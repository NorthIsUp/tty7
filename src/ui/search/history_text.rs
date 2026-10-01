//! The History tab: full text over what was said in past agent sessions
//! ([`ForkHost::search_agent_history`](tty7_core::core::fork_host::ForkHost::search_agent_history)),
//! one row per session with its best match. Enter resumes it where it ran,
//! the way a Sessions row does.
//!
//! It is the Text tab's shape (`text`): its own tab, never on All, asked only
//! after three characters and a pause, the last query's answer alone shown.
//! The history is this machine's, so a remote workspace's tab is empty.

use std::collections::BTreeSet;
use std::path::Path;

use crate::core::history_search::HistoryHit;
use gpui::{Context, Window};

use super::SearchTab;
use super::command::{Avatar, CommandKind, Item};
use super::sources::Catalog;
use super::text::{Ticket, pattern};
use crate::core::agent_history::session_key;
use crate::ui::app::Tty7App;
use crate::ui::host_ops::HostOps;
use crate::ui::i18n::{L10nKey, t, t_fmt};

/// One row per session: the snippet as the title, so the list picks out what
/// was typed in it; the agent and where it ran beside it, and when on the
/// right. Sessions hidden from the Sessions tab stay hidden here.
pub(crate) fn hit_rows(
    hits: Vec<HistoryHit>,
    home: Option<&Path>,
    now: u64,
    hidden: &BTreeSet<String>,
) -> Vec<Item> {
    hits.into_iter()
        .filter(|h| !hidden.contains(&session_key(h.agent, &h.id)))
        .map(|h| {
            let mut subtitle = h.agent.display_name().to_string();
            if let Some(cwd) = &h.cwd {
                subtitle += " · ";
                subtitle += &crate::ui::home::display_path(cwd, home);
            }
            if h.hits > 1 {
                subtitle += " · ";
                subtitle += &t_fmt(
                    L10nKey::SearchHistoryHits,
                    &[("count", &h.hits.to_string())],
                );
            }
            Item::new(
                h.snippet,
                CommandKind::ResumeSession {
                    agent: h.agent,
                    session_id: h.id,
                    cwd: h.cwd,
                },
            )
            .with_subtitle(subtitle)
            .with_note(crate::ui::home::relative_time(now, h.updated))
            .with_avatar(Avatar {
                agent: Some(h.agent),
                ..Avatar::default()
            })
        })
        .collect()
}

/// The empty tab's hint: another machine's workspace, type more, or type
/// something else.
pub(crate) fn empty_hint(catalog: &Catalog, query: &str) -> &'static str {
    if !catalog.live(SearchTab::History).is_some_and(|l| l.asks()) {
        return t(L10nKey::SearchHistoryRemote);
    }
    match pattern(query) {
        Some(_) => t(L10nKey::PaletteTryDifferentSearch),
        None => t(L10nKey::SearchHistoryTooShort),
    }
}

impl Tty7App {
    pub(super) fn palette_history_search(
        &mut self,
        query: String,
        ticket: Ticket,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let land = ticket.clone();
        self.after_pause(
            ticket,
            &query,
            window,
            cx,
            move |app, pattern, window, cx| {
                let (Some(pattern), Some(host)) = (pattern, app.active_host(cx)) else {
                    return app.palette_live_land(land, Vec::new(), window, cx);
                };
                HostOps::run_in(
                    host,
                    window,
                    cx,
                    move |h| h.fork().search_agent_history(&pattern),
                    move |app, found, window, cx| {
                        let found = found.unwrap_or_else(|e| {
                            log::debug!("palette history search: {e}");
                            Vec::new()
                        });
                        let home = crate::ui::path_display::home_for_host(cx, app.spawn_host(cx));
                        let rows = hit_rows(
                            found,
                            home.as_deref(),
                            crate::core::config::unix_now(),
                            &cx.global::<crate::core::config::Config>()
                                .hidden_agent_sessions,
                        );
                        app.palette_live_land(land, rows, window, cx);
                    },
                );
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cli_agent::CLIAgent;
    use crate::ui::search::SearchTab;
    use gpui::TestAppContext;

    fn hit(id: &str, snippet: &str, hits: usize) -> HistoryHit {
        HistoryHit {
            agent: CLIAgent::Claude,
            id: id.into(),
            cwd: Some("/home/me/src/app".into()),
            updated: 100,
            snippet: snippet.into(),
            hits,
            score: 0,
        }
    }

    #[test]
    fn a_hit_is_its_snippet_and_resumes_the_session_where_it_ran() {
        crate::ui::i18n::set_locale("en");
        let rows = hit_rows(
            vec![
                hit("a", "fix the flaky widget test", 3),
                hit("b", "widget", 1),
            ],
            Some(Path::new("/home/me")),
            100,
            &BTreeSet::new(),
        );
        assert_eq!(rows[0].title, "fix the flaky widget test");
        assert_eq!(
            rows[0].subtitle.as_deref(),
            Some("Claude Code · ~/src/app · 3 hits")
        );
        assert_eq!(rows[0].note.as_deref(), Some("just now"));
        assert_eq!(
            rows[0].kind,
            CommandKind::ResumeSession {
                agent: CLIAgent::Claude,
                session_id: "a".into(),
                cwd: Some("/home/me/src/app".into()),
            }
        );
        assert_eq!(rows[1].subtitle.as_deref(), Some("Claude Code · ~/src/app"));
    }

    #[test]
    fn a_hidden_session_stays_hidden() {
        let hidden = BTreeSet::from([session_key(CLIAgent::Claude, "a")]);
        let rows = hit_rows(vec![hit("a", "x", 1), hit("b", "y", 1)], None, 100, &hidden);
        assert_eq!(rows.len(), 1);
    }

    fn with_config(cx: &mut TestAppContext) {
        crate::core::config::pin_test_config_dir();
        cx.update(|cx| {
            cx.set_global(crate::core::config::Config::default());
            crate::ui::i18n::set_locale("en");
        });
    }

    /// History hits never reach the All tab, and a remote workspace's tab
    /// says why it is empty.
    #[gpui::test]
    fn history_hits_are_only_on_their_own_tab(cx: &mut TestAppContext) {
        with_config(cx);
        let mut catalog = Catalog::new(
            vec![Item::new("Split Right", CommandKind::SplitRight)],
            Vec::new(),
            Vec::new(),
        );
        let mut live = crate::ui::search::text::LiveTab::new(SearchTab::History, None);
        live.rows = hit_rows(
            vec![hit("a", "split the pane", 1)],
            None,
            100,
            &BTreeSet::new(),
        );
        catalog.live = vec![live];
        cx.update(|cx| {
            for query in ["", "split"] {
                let headers: Vec<_> = catalog
                    .sections(SearchTab::All, query, cx)
                    .into_iter()
                    .filter_map(|s| s.title)
                    .collect();
                assert!(
                    !headers.iter().any(|h| h == "History"),
                    "no history hits on the All tab for {query:?}: {headers:?}"
                );
            }
            assert_eq!(
                catalog.sections(SearchTab::History, "split", cx)[0]
                    .rows
                    .len(),
                1
            );
            assert!(catalog.sections(SearchTab::History, "sp", cx).is_empty());
        });
        assert_eq!(
            empty_hint(&catalog, "split"),
            "Agent history is searched on this computer only."
        );
    }
}
