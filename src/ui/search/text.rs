//! The Text tab: find in files from Search Everywhere, over the same roots
//! and the same engine as the right panel's "In file contents" search
//! ([`Host::search_content`](tty7_core::host::Host::search_content)), so a
//! remote workspace is searched on its own machine.
//!
//! Every query reads files, which is why the tab is never part of the All
//! tab (`Catalog::all` leaves it out) and why nothing is asked until the
//! query is [`MIN_QUERY_CHARS`] long and typing has paused for [`DEBOUNCE`].
//! Only the answer to the last query asked is ever shown.
//!
//! [`LiveTab`] is that plumbing for any tab answered this way, History
//! (`history_text`) included.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context, Window};
use tty7_core::host::{ContentHit, ContentLimits, ContentQuery};

use super::SearchTab;
use super::command::{CommandKind, Item};
use super::sources::{Catalog, Section, Source};
use super::view::SearchView;
use crate::ui::app::Tty7App;
use crate::ui::host_ops::HostOps;
use crate::ui::i18n::{L10nKey, t};
use crate::ui::panel_search::model::split_relative;

/// Shorter queries match nearly every line of a project, which is a walk of
/// the whole tree for a list nobody reads.
pub(crate) const MIN_QUERY_CHARS: usize = 3;

/// The panel's pause: every query that gets through reads files.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// Far fewer than the panel's cap: the palette is a list to pick one line
/// from, not a report, and a per-file cap keeps one noisy file from filling
/// it.
fn limits() -> ContentLimits {
    ContentLimits {
        max_hits: 200,
        max_hits_per_file: 20,
        max_millis: 5_000,
        ..ContentLimits::default()
    }
}

/// The text a query searches for, or `None` while it is too short to ask.
pub(crate) fn pattern(query: &str) -> Option<&str> {
    let query = query.trim();
    (query.chars().count() >= MIN_QUERY_CHARS).then_some(query)
}

/// One row per hit: the line as the title, so the list's own highlight picks
/// out what was typed, and `dir/file:line` on the right.
pub(crate) fn hit_rows(hits: &[ContentHit], roots: &[PathBuf]) -> Vec<Item> {
    hits.iter()
        .map(|hit| {
            let (name, dir) = split_relative(&hit.path, roots);
            let place = match dir.is_empty() {
                true => format!("{name}:{}", hit.line),
                false => format!("{dir}/{name}:{}", hit.line),
            };
            Item::new(
                hit.text.clone(),
                CommandKind::OpenFile {
                    path: hit.path.clone(),
                    line: Some(hit.line),
                    column: Some(hit.column),
                },
            )
            .with_subtitle(place)
        })
        .collect()
}

/// The empty tab's hint: type more, or type something else.
pub(crate) fn empty_hint(query: &str) -> &'static str {
    match pattern(query) {
        Some(_) => t(L10nKey::PaletteTryDifferentSearch),
        None => t(L10nKey::SearchTextTooShort),
    }
}

/// A tab whose rows the window answers for (Text, History): the last
/// answer's rows, what asks for the next, and which ask is the latest.
#[derive(Clone)]
pub(crate) struct LiveTab {
    pub tab: SearchTab,
    pub rows: Vec<Item>,
    /// `None` where the tab cannot ask: History in a remote workspace.
    ask: Option<LiveAsk>,
    /// Shared by every copy of this palette's catalog and by the tickets it
    /// hands out, so an answer is judged against its own palette only.
    latest: Rc<Cell<u64>>,
}

/// Asks the window for a query's rows; the answer comes back with the ticket
/// through [`SearchView::set_live_rows`].
pub(crate) type LiveAsk = Rc<dyn Fn(&str, Ticket, &mut App)>;

/// One ask of one tab in one palette.
#[derive(Clone)]
pub(crate) struct Ticket {
    seq: u64,
    latest: Rc<Cell<u64>>,
}

impl Ticket {
    /// Nothing newer has been asked of its tab since.
    pub(crate) fn current(&self) -> bool {
        self.latest.get() == self.seq
    }
}

impl LiveTab {
    pub(crate) fn new(tab: SearchTab, ask: Option<LiveAsk>) -> Self {
        Self {
            tab,
            rows: Vec::new(),
            ask,
            latest: Rc::default(),
        }
    }

    pub(crate) fn asks(&self) -> bool {
        self.ask.is_some()
    }

    /// A ticket that makes every earlier one stale.
    fn next(&self) -> Ticket {
        self.latest.set(self.latest.get() + 1);
        Ticket {
            seq: self.latest.get(),
            latest: self.latest.clone(),
        }
    }

    /// `ticket` is this tab's latest ask.
    fn owns(&self, ticket: &Ticket) -> bool {
        Rc::ptr_eq(&self.latest, &ticket.latest) && ticket.current()
    }
}

/// The rows of the last search that came back, for the tab they belong to.
/// Until the next one lands, they stay filtered by what is typed now, so
/// extending a query narrows the list at once instead of blanking it.
pub(super) struct Text<'a>(pub &'a [Item], pub SearchTab);

impl Source for Text<'_> {
    fn tab(&self) -> SearchTab {
        self.1
    }

    fn browse(&self, _cx: &App) -> Vec<Section> {
        Vec::new()
    }

    fn highlights(&self, _cx: &App) -> Vec<Item> {
        Vec::new()
    }

    fn on_the_empty_all_tab(&self) -> bool {
        false
    }

    fn search(&self, query: &str, _cx: &App) -> Vec<(i32, Item)> {
        let Some(pattern) = pattern(query) else {
            return Vec::new();
        };
        let needle = pattern.to_lowercase();
        self.0
            .iter()
            .filter(|item| item.title.to_lowercase().contains(&needle))
            .map(|item| (0, item.clone()))
            .collect()
    }
}

impl Catalog {
    pub(crate) fn live(&self, tab: SearchTab) -> Option<&LiveTab> {
        self.live.iter().find(|l| l.tab == tab)
    }

    /// `tab`'s rows as a source, when the window answers for it.
    pub(super) fn live_source(&self, tab: SearchTab) -> Option<Box<dyn Source + '_>> {
        let rows = self.live(tab).map_or(&[][..], |l| &l.rows);
        Some(Box::new(Text(rows, tab)))
    }

    /// Asks the window for `tab`'s rows for `query`, if the window answers
    /// for that tab.
    pub(super) fn ask_live(&self, tab: SearchTab, query: &str, cx: &mut App) {
        if let Some(live) = self.live(tab)
            && let Some(ask) = live.ask.clone()
        {
            ask(query, live.next(), cx);
        }
    }
}

impl SearchView {
    /// A live tab's answer; dropped unless `ticket` is its latest ask here.
    pub(crate) fn set_live_rows(
        &mut self,
        ticket: Ticket,
        rows: Vec<Item>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_catalog(
            |catalog| {
                if let Some(live) = catalog.live.iter_mut().find(|l| l.owns(&ticket)) {
                    live.rows = rows;
                }
            },
            window,
            cx,
        );
    }
}

impl Tty7App {
    /// The tabs this window answers as they are typed in: Text, and History
    /// when the workspace is on this machine (the history searched is its).
    pub(crate) fn palette_live_tabs(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<LiveTab> {
        let history = self.spawn_host(cx).is_local();
        vec![
            LiveTab::new(
                SearchTab::Text,
                Some(self.live_ask(window, cx, Self::palette_text_search)),
            ),
            LiveTab::new(
                SearchTab::History,
                history.then(|| self.live_ask(window, cx, Self::palette_history_search)),
            ),
        ]
    }

    /// An ask that runs `search` on this window, outside the search's own
    /// update.
    fn live_ask(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        search: fn(&mut Self, String, Ticket, &mut Window, &mut Context<Self>),
    ) -> LiveAsk {
        let app = cx.entity().downgrade();
        let handle = window.window_handle();
        Rc::new(move |query: &str, ticket: Ticket, cx: &mut App| {
            let (app, query) = (app.clone(), query.to_owned());
            // Deferred: the query arrives from inside the search's own
            // update, and answering it updates the search.
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = app.update(cx, |app, cx| search(app, query, ticket, window, cx));
                });
            });
        })
    }

    /// Once typing has paused with nothing newer asked, runs `ask` with the
    /// query's pattern. A query too short to ask gets `None` at once, so the
    /// tab empties without waiting.
    pub(super) fn after_pause(
        &mut self,
        ticket: Ticket,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        ask: impl FnOnce(&mut Self, Option<String>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(pattern) = pattern(query).map(str::to_owned) else {
            return ask(self, None, window, cx);
        };
        cx.spawn_in(window, async move |app, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            if !ticket.current() {
                return;
            }
            let _ = app.update_in(cx, |app, window, cx| ask(app, Some(pattern), window, cx));
        })
        .detach();
    }

    /// Hands a live tab's answer to the open search.
    pub(super) fn palette_live_land(
        &mut self,
        ticket: Ticket,
        rows: Vec<Item>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.search.clone().filter(|_| ticket.current()) {
            view.update(cx, |view, cx| view.set_live_rows(ticket, rows, window, cx));
        }
    }

    fn palette_text_search(
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
                let Some(pattern) = pattern else {
                    return app.palette_live_land(land, Vec::new(), window, cx);
                };
                // After the pause, not before: a tab whose repository root was
                // not yet known has had the time to resolve it.
                // ponytail: still resolving after the pause finds nothing; the
                // next keystroke asks again.
                let roots = app.project_roots(cx).filter(|(_, roots)| !roots.is_empty());
                let Some((host, roots)) = roots else {
                    return app.palette_live_land(land, Vec::new(), window, cx);
                };
                let query = ContentQuery {
                    pattern,
                    ..ContentQuery::default()
                };
                HostOps::run_in(
                    host,
                    window,
                    cx,
                    move |h| {
                        let found = h.search_content(&roots, &query, &limits());
                        (found, roots)
                    },
                    move |app, (found, roots), window, cx| {
                        let rows = match found {
                            Ok(found) => hit_rows(&found.hits, &roots),
                            Err(e) => {
                                log::debug!("palette text search: {e}");
                                Vec::new()
                            }
                        };
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
    use gpui::TestAppContext;

    fn hit(path: &str, line: u32, text: &str) -> ContentHit {
        ContentHit {
            path: path.into(),
            line,
            column: 5,
            text: text.into(),
            ranges: Vec::new(),
        }
    }

    #[test]
    fn a_query_is_not_asked_until_it_is_three_characters_long() {
        assert_eq!(pattern(""), None);
        assert_eq!(pattern("fn"), None);
        assert_eq!(pattern("  fn  "), None);
        assert_eq!(pattern("fn "), None);
        assert_eq!(pattern("fn x"), Some("fn x"));
        assert_eq!(pattern("日本語"), Some("日本語"));
    }

    #[test]
    fn a_hit_is_its_line_and_opens_the_file_there() {
        let roots = vec![PathBuf::from("/repo")];
        let rows = hit_rows(
            &[
                hit("/repo/src/main.rs", 12, "fn main() {"),
                hit("/repo/README.md", 3, "main entry"),
            ],
            &roots,
        );
        assert_eq!(rows[0].title, "fn main() {");
        assert_eq!(rows[0].subtitle.as_deref(), Some("src/main.rs:12"));
        assert_eq!(
            rows[0].kind,
            CommandKind::OpenFile {
                path: "/repo/src/main.rs".into(),
                line: Some(12),
                column: Some(5),
            }
        );
        assert_eq!(rows[1].subtitle.as_deref(), Some("README.md:3"));
    }

    /// Each palette numbers its own asks, so a query typed in one window's
    /// palette never makes another window's pending answer stale.
    #[test]
    fn an_ask_in_one_palette_leaves_anothers_answer_current() {
        let a = LiveTab::new(SearchTab::Text, None);
        let b = LiveTab::new(SearchTab::Text, None);
        let first = a.next();
        let other = b.next();
        assert!(first.current() && a.owns(&first));
        assert!(
            !a.owns(&other),
            "an answer lands only in the palette that asked"
        );

        let copy = a.clone();
        let second = copy.next();
        assert!(!first.current(), "a newer ask, even through a catalog copy");
        assert!(a.owns(&second) && other.current());
    }

    fn catalog_with_hits() -> Catalog {
        let mut catalog = Catalog::new(
            vec![Item::new("Split Right", CommandKind::SplitRight)],
            Vec::new(),
            Vec::new(),
        );
        let mut live = LiveTab::new(SearchTab::Text, None);
        live.rows = hit_rows(
            &[
                hit("/repo/a.rs", 1, "let split_point = 3;"),
                hit("/repo/b.rs", 2, "split right here"),
            ],
            &[PathBuf::from("/repo")],
        );
        catalog.live = vec![live];
        catalog
    }

    fn with_config(cx: &mut TestAppContext) {
        crate::core::config::pin_test_config_dir();
        cx.update(|cx| {
            cx.set_global(crate::core::config::Config::default());
            crate::ui::i18n::set_locale("en");
        });
    }

    /// Text hits never reach the All tab, typed or not, even when the Text
    /// tab has rows that match what is typed there.
    #[gpui::test]
    fn text_hits_are_only_on_their_own_tab(cx: &mut TestAppContext) {
        with_config(cx);
        let catalog = catalog_with_hits();
        cx.update(|cx| {
            for query in ["", "split"] {
                let headers: Vec<_> = catalog
                    .sections(SearchTab::All, query, cx)
                    .into_iter()
                    .filter_map(|s| s.title)
                    .collect();
                assert!(
                    !headers.iter().any(|h| h == "Text"),
                    "no text hits on the All tab for {query:?}: {headers:?}"
                );
            }
            let own = catalog.sections(SearchTab::Text, "split", cx);
            assert_eq!(own[0].rows.len(), 2);
        });
    }

    /// Typing past the last answer narrows it at once; a query too short to
    /// ask shows nothing rather than the last answer.
    #[gpui::test]
    fn the_last_answer_narrows_to_what_is_typed_now(cx: &mut TestAppContext) {
        with_config(cx);
        let catalog = catalog_with_hits();
        cx.update(|cx| {
            let rows = catalog.sections(SearchTab::Text, "SPLIT_P", cx);
            let titles: Vec<_> = rows[0]
                .rows
                .iter()
                .filter_map(|r| r.item().map(|i| i.title.clone()))
                .collect();
            assert_eq!(titles, vec!["let split_point = 3;"]);
            assert!(catalog.sections(SearchTab::Text, "sp", cx).is_empty());
            assert!(catalog.sections(SearchTab::Text, "", cx).is_empty());
        });
    }

    /// End to end on this machine: the tab's pane reports a directory, the
    /// Text tab is opened with a query, and after the pause the hits of that
    /// directory land as rows that open the file on the matching line.
    #[gpui::test]
    fn a_query_on_the_text_tab_lands_the_projects_matching_lines(cx: &mut TestAppContext) {
        use crate::daemon::protocol::DaemonMsg;
        use crate::ui::app::test_window;

        let root = std::env::temp_dir().join(format!("tty7-ptext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        let root = std::fs::canonicalize(&root).unwrap();
        std::fs::write(root.join("src/lib.rs"), "let x = 1;\nfn needle() {}\n").unwrap();

        let (app, mut vcx, mut pane) = test_window::harness_with_pane(cx);
        DaemonMsg::Cwd(root.clone())
            .encode(&mut pane)
            .expect("the pane's socket takes the cwd");
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let wait = |vcx: &mut gpui::VisualTestContext| {
            vcx.executor().advance_clock(DEBOUNCE);
            vcx.background_executor.run_until_parked();
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(20));
        };
        while app.update_in(&mut vcx, |app, _, cx| {
            app.project_roots(cx).map(|(_, roots)| roots) != Some(vec![root.clone()])
        }) {
            wait(&mut vcx);
        }

        app.update_in(&mut vcx, |app, window, cx| {
            app.open_search(SearchTab::Text, "needle", window, cx)
        });
        let rows = loop {
            let rows = app.update_in(&mut vcx, |app, _, cx| {
                app.search.as_ref().map(|v| v.read(cx).text_rows())
            });
            if let Some(rows) = rows.filter(|r| !r.is_empty()) {
                break rows;
            }
            wait(&mut vcx);
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "fn needle() {}");
        assert_eq!(rows[0].subtitle.as_deref(), Some("src/lib.rs:2"));
        assert_eq!(
            rows[0].kind,
            CommandKind::OpenFile {
                path: root.join("src/lib.rs"),
                line: Some(2),
                column: Some(4),
            }
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
