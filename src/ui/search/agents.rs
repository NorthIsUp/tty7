//! Search Everywhere's Agents tab (⌘K): every open tab, then every agent
//! session not already open in one. Both halves are the Terminals and
//! Sessions tabs' own rows, so Enter focuses or resumes exactly as there.

use gpui::App;

use super::SearchTab;
use super::command::{CommandKind, Item};
use super::sources::{Section, Source, by_section, rank};
use crate::core::session::WorkspaceStore;
use crate::ui::app::Tty7App;
use crate::ui::machine_mirror::MachineMirrors;

pub(super) struct Agents<'a> {
    pub terminals: &'a [Item],
    pub sessions: &'a [Item],
    pub open: &'a [String],
}

impl Agents<'_> {
    /// Open tabs in the Terminals tab's order (this window's by MRU, then
    /// the other workspaces'), without its New Terminal rows.
    fn tabs(&self) -> Vec<Item> {
        self.terminals
            .iter()
            .filter(|item| matches!(item.kind, CommandKind::GoToTab { .. }))
            .cloned()
            .collect()
    }

    /// Sessions in the Sessions tab's order, less those open in a pane: that
    /// one is its tab's row, and resuming it would run a second copy.
    fn resumable(&self) -> Vec<Item> {
        self.sessions
            .iter()
            .filter(|item| match &item.kind {
                CommandKind::ResumeSession { session_id, .. } => !self.open.contains(session_id),
                _ => false,
            })
            .cloned()
            .collect()
    }
}

impl Source for Agents<'_> {
    fn tab(&self) -> SearchTab {
        SearchTab::Agents
    }

    fn browse(&self, _cx: &App) -> Vec<Section> {
        by_section(self.tabs().iter().chain(self.resumable().iter()))
    }

    fn highlights(&self, _cx: &App) -> Vec<Item> {
        Vec::new()
    }

    /// Open tabs stay ahead of sessions under a query too; each half ranks
    /// by score on its own.
    fn search(&self, query: &str, _cx: &App) -> Vec<(i32, Item)> {
        let mut hits = rank(&self.tabs(), query, |_| 0);
        hits.extend(rank(&self.resumable(), query, |_| 0));
        hits
    }
}

impl Tty7App {
    /// Session ids of every agent pane this process knows: this window's
    /// panes, and every pane the machine mirrors hold for other windows.
    pub(crate) fn open_agent_session_ids(&self, cx: &App) -> Vec<String> {
        let here = self
            .tabs
            .iter()
            .flat_map(|tab| tab.pane.terminals())
            .filter_map(|leaf| leaf.read(cx).agent_session()?.session_id);
        let mirrored = WorkspaceStore::all(cx)
            .views
            .iter()
            .filter_map(|w| MachineMirrors::machine(cx, w.host_id()))
            .flat_map(|machine| machine.panes.iter())
            .filter_map(|pane| pane.agent.as_ref()?.session_id.clone());
        here.chain(mirrored).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cli_agent::CLIAgent;
    use gpui::TestAppContext;
    use tty7_core::core::machine::TabId;
    use tty7_core::core::session::WorkspaceId;

    fn tab(title: &str) -> Item {
        Item::new(
            title,
            CommandKind::GoToTab {
                workspace: WorkspaceId::new(),
                tab: TabId::new(),
            },
        )
    }

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

    fn titles(rows: Vec<Item>) -> Vec<String> {
        rows.into_iter().map(|i| i.title).collect()
    }

    fn fixture() -> (Vec<Item>, Vec<Item>, Vec<String>) {
        let terminals = vec![
            tab("api server"),
            tab("claude fixing tests"),
            Item::new("New zsh", CommandKind::NewTab),
        ];
        let sessions = vec![
            session("fixing tests", "open-1"),
            session("tests for the parser", "past-1"),
            session("docs pass", "past-2"),
        ];
        (terminals, sessions, vec!["open-1".into()])
    }

    #[gpui::test]
    fn open_tabs_then_sessions_not_open_in_one(cx: &mut TestAppContext) {
        let (terminals, sessions, open) = fixture();
        let agents = Agents {
            terminals: &terminals,
            sessions: &sessions,
            open: &open,
        };
        cx.update(|cx| {
            let rows: Vec<Item> = agents
                .browse(cx)
                .into_iter()
                .flat_map(|s| s.rows)
                .filter_map(|r| r.item().cloned())
                .collect();
            assert_eq!(
                titles(rows),
                [
                    "api server",
                    "claude fixing tests",
                    "tests for the parser",
                    "docs pass"
                ]
            );

            // A session scoring higher than a tab still comes after it.
            let hits = agents.search("tests", cx).into_iter().map(|(_, i)| i);
            assert_eq!(
                titles(hits.collect()),
                ["claude fixing tests", "tests for the parser"]
            );
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    fn cmd_k_opens_the_agents_tab_and_closes_it_again(cx: &mut TestAppContext) {
        let (app, mut vcx, _streams) = crate::ui::app::test_window::harness_with_tabs(cx, 2);
        app.update_in(&mut vcx, |app, window, cx| app.focus_active(window, cx));
        vcx.simulate_keystrokes("cmd-k");
        vcx.run_until_parked();
        let view = app.read_with(&vcx, |app, _| {
            app.search.clone().expect("⌘K opens the search")
        });
        view.read_with(&vcx, |view, _| assert_eq!(view.tab(), SearchTab::Agents));

        vcx.simulate_keystrokes("cmd-k");
        vcx.run_until_parked();
        assert!(app.read_with(&vcx, |app, _| app.search.is_none()));
    }
}
