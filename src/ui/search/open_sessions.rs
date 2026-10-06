//! The Sessions tab's lead (⌘K): the agent sessions open in a tab, each as a
//! row that goes to that tab. Resuming one there would run a second copy.

use std::collections::HashMap;

use gpui::App;
use tty7_core::core::machine::TabId;

use super::command::{CommandKind, Item};
use crate::core::session::{WorkspaceId, WorkspaceStore};
use crate::ui::app::Tty7App;
use crate::ui::i18n::{L10nKey, t};

pub(crate) type OpenSessions = HashMap<String, (WorkspaceId, TabId)>;

/// `rows` with the sessions in `open` moved to the front under their own
/// heading, each going to its tab; the rest keep their order. `here` counts
/// the rows that ran where you are, and comes back counting the open ones
/// ahead of them too.
pub(crate) fn open_first(rows: Vec<Item>, here: usize, open: &OpenSessions) -> (Vec<Item>, usize) {
    let section: gpui::SharedString = t(L10nKey::SearchSectionSessionsOpen).into();
    let (mut lead, mut rest) = (Vec::new(), Vec::new());
    let mut here_left = here;
    for (at, mut item) in rows.into_iter().enumerate() {
        let tab = match &item.kind {
            CommandKind::ResumeSession { session_id, .. } => open.get(session_id).copied(),
            _ => None,
        };
        match tab {
            Some((workspace, tab)) => {
                item.kind = CommandKind::GoToTab { workspace, tab };
                lead.push(item.in_section(section.clone()));
                if at < here {
                    here_left -= 1;
                }
            }
            None => rest.push(item),
        }
    }
    let here = lead.len() + here_left;
    lead.extend(rest);
    (lead, here)
}

impl Tty7App {
    /// Every agent session open in a pane this process knows, with its tab:
    /// this window's panes, and every pane the machine mirrors hold for other
    /// windows.
    pub(crate) fn open_agent_sessions(&self, cx: &App) -> OpenSessions {
        let mut out = OpenSessions::new();
        for w in &WorkspaceStore::all(cx).views {
            for (session, tab) in crate::ui::machine_mirror::agent_sessions_of(cx, w) {
                out.insert(session, (w.id, tab));
            }
        }
        // This window's own panes last: they are current where a mirror may lag.
        for tab in &self.tabs {
            for leaf in tab.pane.terminals() {
                if let Some(session) = leaf.read(cx).agent_session().and_then(|s| s.session_id) {
                    out.insert(session, (self.workspace, tab.tree_id.get()));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cli_agent::CLIAgent;
    use crate::ui::search::SearchTab;
    use gpui::TestAppContext;

    fn session(title: &str, id: &str) -> Item {
        Item::new(
            title,
            CommandKind::ResumeSession {
                agent: CLIAgent::Claude,
                session_id: id.into(),
                cwd: None,
            },
        )
    }

    #[gpui::test]
    fn open_sessions_lead_and_go_to_their_tab(cx: &mut TestAppContext) {
        let (ws, tab) = (WorkspaceId::new(), TabId::new());
        let open = OpenSessions::from([
            ("open-here".to_string(), (ws, tab)),
            ("open-elsewhere".to_string(), (ws, tab)),
        ]);
        let rows = vec![
            session("here, past", "past-here"),
            session("here, open", "open-here"),
            session("recent, past", "past"),
            session("recent, open", "open-elsewhere"),
        ];
        let (rows, here) = cx.update(|_| open_first(rows, 2, &open));
        let titles: Vec<_> = rows.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(
            titles,
            ["here, open", "recent, open", "here, past", "recent, past"]
        );
        assert_eq!(here, 3, "both open rows, then the one past row from here");
        assert!(
            rows[..2]
                .iter()
                .all(|i| i.kind == CommandKind::GoToTab { workspace: ws, tab })
        );
        assert!(matches!(rows[2].kind, CommandKind::ResumeSession { .. }));
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn cmd_k_opens_the_sessions_tab_and_closes_it_again(cx: &mut TestAppContext) {
        let (app, mut vcx, _streams) = crate::ui::app::test_window::harness_with_tabs(cx, 2);
        app.update_in(&mut vcx, |app, window, cx| app.focus_active(window, cx));
        vcx.simulate_keystrokes("cmd-k");
        vcx.run_until_parked();
        let view = app.read_with(&vcx, |app, _| {
            app.search.clone().expect("⌘K opens the search")
        });
        view.read_with(&vcx, |view, _| assert_eq!(view.tab(), SearchTab::Sessions));

        vcx.simulate_keystrokes("cmd-k");
        vcx.run_until_parked();
        assert!(app.read_with(&vcx, |app, _| app.search.is_none()));
    }
}
