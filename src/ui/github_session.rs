//! The GitHub tab's Session list, its default: every issue and pull request
//! the focused pane's agent session mentions
//! ([`ForkHost::agent_session_mentions`](tty7_core::core::fork_host::ForkHost::agent_session_mentions)),
//! latest mention first, under the same Open | Closed | All switch as the
//! other lists.
//!
//! A mention's row comes from whatever the panel already holds for that
//! number: a list page, a detail, or an earlier lookup. The Session tab reads
//! the repository's `state=all` issue and pull request lists (two requests),
//! then looks up the newest [`MAX_LOOKUPS`] still unknown one at a time on a
//! single worker. Anything past that shows as a bare `#N` under All.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use gpui::{AnyElement, Context, SharedString, Window, div, prelude::*, px, rems};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use tty7_core::core::cli_agent::CLIAgent;
use tty7_core::core::github::api::{self, ListQuery};
use tty7_core::core::github::{ApiError, Item, Kind, RepoSlug, StateFilter};
use tty7_core::core::history_search::Mentions;
use tty7_core::host::HostId;

use crate::ui::app::{CONTENT_INSET, Tty7App};
use crate::ui::github::{STALE_AFTER, off_ui};
use crate::ui::host_ops::{HostOps, SharedHost};
use crate::ui::i18n::{L10nKey, t};
use crate::ui::right_panel::{META, ROW_FILL_RADIUS, ROW_INSET};

/// ponytail: mentions past the newest this many are never looked up one by
/// one; they stay `#N` under All unless a list page or a detail has them.
const MAX_LOOKUPS: usize = 30;

/// Which session's mentions, of which repository.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SessionKey {
    host: HostId,
    agent: CLIAgent,
    id: String,
    slug: RepoSlug,
}

pub(crate) struct SessionCache {
    key: SessionKey,
    mentions: Option<Arc<Mentions>>,
    loading: bool,
    fetched: Option<Instant>,
}

pub(crate) struct SessionState {
    /// The Session tab is showing, instead of `kind`'s list.
    pub(crate) tab: bool,
    cache: Option<SessionCache>,
    /// Rows read one at a time; `None` for a number GitHub has no issue or
    /// pull request under.
    looked_up: HashMap<(RepoSlug, u64), Option<Item>>,
    /// The lookup in flight per repository, by the `seq` it started under.
    looking_up: HashMap<RepoSlug, u64>,
    /// A lookup failed short of a 404 (rate limit, 401, network): the
    /// repository waits [`STALE_AFTER`] before the next one.
    backoff: HashMap<RepoSlug, Instant>,
    /// Bumped by refresh and by the Session list moving to another
    /// repository; a worker stops between items once it moves, and its
    /// results are dropped.
    seq: Arc<AtomicU64>,
    shown: Option<RepoSlug>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            tab: true,
            cache: None,
            looked_up: HashMap::new(),
            looking_up: HashMap::new(),
            backoff: HashMap::new(),
            seq: Arc::new(AtomicU64::new(0)),
            shown: None,
        }
    }
}

/// Refresh reads the session again, and every looked-up row.
pub(crate) fn mark_due(session: &mut SessionState) {
    if let Some(c) = &mut session.cache {
        c.fetched = None;
    }
    session.seq.fetch_add(1, Ordering::Relaxed);
    session.looked_up.clear();
    session.looking_up.clear();
    session.backoff.clear();
}

/// Per number, the row read last (highest `updated_at`) among every source.
fn newest<'a>(items: impl Iterator<Item = &'a Item>) -> HashMap<u64, &'a Item> {
    let mut out: HashMap<u64, &Item> = HashMap::new();
    for item in items {
        out.entry(item.number)
            .and_modify(|had| {
                if item.updated_at > had.updated_at {
                    *had = item;
                }
            })
            .or_insert(item);
    }
    out
}

/// The remote to prefer: the user's pick, else `origin` (the fork) when
/// `prefer_origin`, else upstream's own order in `default_remote`.
pub(crate) fn remote_pick(pick: Option<&str>, prefer_origin: bool) -> Option<&str> {
    pick.or(prefer_origin.then_some("origin"))
}

/// A Session row: the item, or just its number while nothing says more.
#[derive(Debug, PartialEq)]
enum Row<'a> {
    Known(&'a Item),
    Unknown(u64),
}

/// `numbers` (latest mention first) as rows, those `state` and `label` keep.
/// An unknown number has no state or labels to go by, so only All with no
/// label shows it.
fn session_rows<'a>(
    numbers: &[u64],
    known: &HashMap<u64, &'a Item>,
    state: StateFilter,
    label: Option<&str>,
) -> Vec<Row<'a>> {
    numbers
        .iter()
        .filter_map(|n| match known.get(n) {
            Some(&item) => (state.admits(item.state)
                && label.is_none_or(|l| item.labels.iter().any(|x| x.name == l)))
            .then_some(Row::Known(item)),
            None => (state == StateFilter::All && label.is_none()).then_some(Row::Unknown(*n)),
        })
        .collect()
}

/// The first [`MAX_LOOKUPS`] mentions nothing has a row for yet.
fn to_look_up(
    numbers: &[u64],
    known: &HashMap<u64, &Item>,
    tried: impl Fn(u64) -> bool,
) -> Vec<u64> {
    numbers
        .iter()
        .take(MAX_LOOKUPS)
        .copied()
        .filter(|n| !known.contains_key(n) && !tried(*n))
        .collect()
}

fn all_query(slug: &RepoSlug, kind: Kind) -> ListQuery {
    ListQuery {
        slug: slug.clone(),
        kind,
        state: StateFilter::All,
        label: None,
    }
}

impl Tty7App {
    /// The Session cell, first of Session | Issues | Pull Requests.
    pub(crate) fn github_session_tab_cell(&self, cx: &mut Context<Self>) -> AnyElement {
        crate::ui::panel_github::switch_cell(
            ("panel-github-session", 0),
            t(L10nKey::GitHubSession),
            self.github.session.tab,
            cx,
        )
        .on_click(cx.listener(|this, _, _window, cx| {
            this.github.session.tab = true;
            this.github.list_scroll = gpui::ScrollHandle::new();
            cx.notify();
        }))
        .into_any_element()
    }

    /// The Session list, or `None` on the Issues and Pull Requests tabs.
    pub(crate) fn github_session_tab_body(
        &mut self,
        host: &SharedHost,
        slug: &RepoSlug,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.github.session.tab {
            return None;
        }
        let mentions = match self.github_session_key(host, slug, window, cx) {
            None => Some(Arc::new(Mentions::default())),
            Some(key) => self.github_ensure_mentions(host, key, cx),
        };
        let Some(mentions) = mentions else {
            return Some(self.panel_empty(t(L10nKey::PanelLoading), None, cx));
        };
        if mentions.numbers.is_empty() {
            return Some(self.panel_empty(t(L10nKey::GitHubNoSessionMentions), None, cx));
        }
        if self.github.session.shown.as_ref() != Some(slug) {
            self.github.session.shown = Some(slug.clone());
            self.github.session.seq.fetch_add(1, Ordering::Relaxed);
        }
        self.github_session_lists(slug, cx);
        let known = self.github_known(slug);
        let missing = self.github_session_missing(slug, &mentions.numbers, &known);
        let rows = session_rows(
            &mentions.numbers,
            &known,
            self.github.state,
            self.github.label.as_deref(),
        );
        let body = if rows.is_empty() {
            self.panel_empty(t(L10nKey::GitHubNoSessionMatches), None, cx)
        } else {
            let now = crate::ui::github::now_unix();
            let mut list = v_flex().px(px(CONTENT_INSET));
            for row in rows {
                list = list.child(match row {
                    Row::Known(item) => self.github_item_row(slug, item, now, cx),
                    Row::Unknown(n) => self.github_unknown_row(slug, n, cx),
                });
            }
            v_flex().pb(px(12.)).child(list).into_any_element()
        };
        drop(known);
        self.github_look_up(slug.clone(), missing, cx);
        Some(body)
    }

    /// The repository's `state=all` lists, which answer most mentions.
    fn github_session_lists(&mut self, slug: &RepoSlug, cx: &mut Context<Self>) {
        for kind in [Kind::Issues, Kind::Pulls] {
            self.github_ensure_list(&all_query(slug, kind), cx);
        }
    }

    /// The mentions to look up one by one now: none until both lists have
    /// landed cleanly, while one is in flight or backing off, or signed out
    /// (60 requests an hour is the whole panel's budget).
    fn github_session_missing(
        &self,
        slug: &RepoSlug,
        numbers: &[u64],
        known: &HashMap<u64, &Item>,
    ) -> Vec<u64> {
        let session = &self.github.session;
        let lists_in = [Kind::Issues, Kind::Pulls].iter().all(|&k| {
            self.github
                .lists
                .get(&all_query(slug, k))
                .is_some_and(|l| l.loaded && !l.loading && l.error.is_none())
        });
        let signed_in = self
            .github
            .connection
            .as_ref()
            .is_some_and(|c| c.transport.authenticated());
        if !lists_in
            || !signed_in
            || session.looking_up.contains_key(slug)
            || session
                .backoff
                .get(slug)
                .is_some_and(|t| t.elapsed() < STALE_AFTER)
        {
            return Vec::new();
        }
        to_look_up(numbers, known, |n| {
            session.looked_up.contains_key(&(slug.clone(), n))
        })
    }

    /// Every row the panel holds for `slug` (lists, lookups, details), the
    /// newest read of each number.
    fn github_known(&self, slug: &RepoSlug) -> HashMap<u64, &Item> {
        let lists = self
            .github
            .lists
            .iter()
            .filter(|(q, _)| &q.slug == slug)
            .flat_map(|(_, l)| l.items.iter());
        let looked_up = self
            .github
            .session
            .looked_up
            .iter()
            .filter(|((s, _), _)| s == slug)
            .filter_map(|(_, i)| i.as_ref());
        let details = self
            .github
            .details
            .iter()
            .filter(|((s, _), _)| s == slug)
            .filter_map(|(_, d)| d.detail.as_ref().map(|d| &d.item));
        newest(lists.chain(looked_up).chain(details))
    }

    /// Read `numbers`' rows one after another on one worker, stopping at
    /// the first failure other than a 404 or once `seq` moves on.
    fn github_look_up(&mut self, slug: RepoSlug, numbers: Vec<u64>, cx: &mut Context<Self>) {
        if numbers.is_empty() {
            return;
        }
        let Some(connection) = self.github_connection(cx) else {
            return;
        };
        let seq = self.github.session.seq.clone();
        let mine = seq.load(Ordering::Relaxed);
        self.github.session.looking_up.insert(slug.clone(), mine);
        cx.spawn(async move |this, cx| {
            let transport = connection.transport.clone();
            let asked = slug.clone();
            let got = off_ui(move || {
                let mut got = Vec::new();
                for n in numbers {
                    if seq.load(Ordering::Relaxed) != mine {
                        return (got, false);
                    }
                    match api::item(&*transport, &asked, n) {
                        Ok(item) => got.push((n, Some(item))),
                        Err(ApiError::NotFound) => got.push((n, None)),
                        Err(e) => {
                            log::warn!("github: {}#{n}: {e}", asked.full());
                            return (got, true);
                        }
                    }
                }
                (got, false)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                let session = &mut this.github.session;
                if session.looking_up.get(&slug) == Some(&mine) {
                    session.looking_up.remove(&slug);
                }
                if session.seq.load(Ordering::Relaxed) != mine {
                    return;
                }
                let (got, failed) = got.unwrap_or_default();
                if failed {
                    session.backoff.insert(slug.clone(), Instant::now());
                }
                for (n, item) in got {
                    session.looked_up.insert((slug.clone(), n), item);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A mention with no row yet: its number, opening its detail on click.
    fn github_unknown_row(
        &self,
        slug: &RepoSlug,
        number: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sf = cx.global::<crate::ui::presets::Surfaces>().sidebar;
        let slug = slug.clone();
        h_flex()
            .id(SharedString::from(format!(
                "panel-github-mentioned-{number}"
            )))
            .items_center()
            .h(px(26.))
            .w_full()
            .px(px(ROW_INSET))
            .rounded(ROW_FILL_RADIUS)
            .cursor_pointer()
            .hover(|s| s.bg(gpui::rgb(sf.hover)))
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.github_open_detail(slug.clone(), number, cx);
            }))
            .child(
                div()
                    .text_size(rems(META))
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("#{number}")),
            )
            .into_any_element()
    }

    /// The focused pane's agent session, if it has one.
    fn github_session_key(
        &self,
        host: &SharedHost,
        slug: &RepoSlug,
        window: &mut Window,
        cx: &gpui::App,
    ) -> Option<SessionKey> {
        let view = self.tabs.get(self.active)?.detail_pane(window, cx)?;
        let view = view.read(cx);
        Some(SessionKey {
            host: host.id(),
            agent: view.agent()?,
            id: view.agent_session()?.session_id?,
            slug: slug.clone(),
        })
    }

    /// `key`'s mentions, read off the UI thread when the session changed or
    /// the last read went stale. `None` until the first read lands.
    fn github_ensure_mentions(
        &mut self,
        host: &SharedHost,
        key: SessionKey,
        cx: &mut Context<Self>,
    ) -> Option<Arc<Mentions>> {
        let same = self
            .github
            .session
            .cache
            .as_ref()
            .is_some_and(|c| c.key == key);
        if !same {
            self.github.session.cache = Some(SessionCache {
                key: key.clone(),
                mentions: None,
                loading: false,
                fetched: None,
            });
        }
        let cache = self.github.session.cache.as_mut()?;
        if !cache.loading && cache.fetched.is_none_or(|t| t.elapsed() > STALE_AFTER) {
            cache.loading = true;
            let asked = key.clone();
            HostOps::run(
                host.clone(),
                cx,
                move |h| {
                    h.fork()
                        .agent_session_mentions(asked.agent, &asked.id, &asked.slug)
                        .unwrap_or_default()
                },
                move |this, mentions, cx| {
                    let Some(c) = this.github.session.cache.as_mut().filter(|c| c.key == key)
                    else {
                        return;
                    };
                    c.loading = false;
                    c.fetched = Some(Instant::now());
                    c.mentions = Some(Arc::new(mentions));
                    cx.notify();
                },
            );
        }
        cache.mentions.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tty7_core::core::github::ItemState;

    fn item(number: u64) -> Item {
        Item {
            number,
            title: format!("pr {number}"),
            state: ItemState::Open,
            is_pr: true,
            author: String::new(),
            labels: Vec::new(),
            comments: 0,
            created_at: 0,
            updated_at: 0,
            html_url: String::new(),
        }
    }

    #[test]
    fn the_fork_is_shown_until_a_remote_is_picked() {
        use tty7_core::core::github::remote::{GitHubRemote, default_remote};
        let remote = |name: &str, owner: &str| GitHubRemote {
            remote: name.into(),
            slug: RepoSlug {
                owner: owner.into(),
                name: "tty7".into(),
            },
        };
        let remotes = [remote("origin", "me"), remote("upstream", "acme")];
        let shown = |pick, prefer| {
            default_remote(&remotes, remote_pick(pick, prefer))
                .unwrap()
                .remote
                .clone()
        };
        assert_eq!(shown(None, true), "origin");
        assert_eq!(shown(None, false), "upstream");
        assert_eq!(shown(Some("upstream"), true), "upstream");
        let only_upstream = [remote("upstream", "acme")];
        assert_eq!(
            default_remote(&only_upstream, remote_pick(None, true))
                .unwrap()
                .remote,
            "upstream"
        );
    }

    #[test]
    fn session_rows_follow_the_mentions_and_the_switch() {
        let mut closed = item(4);
        closed.state = ItemState::Merged;
        let mut issue = item(3);
        issue.is_pr = false;
        let open = item(5);
        let items = [closed, issue, open];
        let known: HashMap<u64, &Item> = items.iter().map(|i| (i.number, i)).collect();
        let numbers = [9, 4, 3, 5];
        let shown = |state, label| -> Vec<String> {
            session_rows(&numbers, &known, state, label)
                .into_iter()
                .map(|r| match r {
                    Row::Known(i) => i.number.to_string(),
                    Row::Unknown(n) => format!("?{n}"),
                })
                .collect()
        };
        assert_eq!(
            shown(StateFilter::All, None),
            ["?9", "4", "3", "5"],
            "latest mention first, issues and pull requests mixed"
        );
        assert_eq!(shown(StateFilter::Open, None), ["3", "5"]);
        assert_eq!(shown(StateFilter::Closed, None), ["4"]);
        assert!(
            shown(StateFilter::All, Some("bug")).is_empty(),
            "an unknown number has no labels"
        );
    }

    #[test]
    fn only_the_newest_unknown_mentions_are_looked_up() {
        let known_item = item(2);
        let known: HashMap<u64, &Item> = [(2, &known_item)].into();
        let numbers: Vec<u64> = (1..=MAX_LOOKUPS as u64 + 10).collect();
        let missing = to_look_up(&numbers, &known, |n| n == 3);
        assert_eq!(missing.len(), MAX_LOOKUPS - 2);
        assert_eq!(
            &missing[..2],
            &[1, 4],
            "known and already tried ones skipped"
        );
    }

    #[test]
    fn the_newest_read_of_a_number_wins_whatever_the_order() {
        let mut stale = item(5);
        stale.updated_at = 1;
        let mut fresh = item(5);
        fresh.updated_at = 2;
        fresh.state = ItemState::Merged;
        for pair in [[&stale, &fresh], [&fresh, &stale]] {
            assert_eq!(newest(pair.into_iter())[&5].state, ItemState::Merged);
        }
    }
}

/// The lookup's lifecycle against a fake GitHub: when it runs, what it
/// keeps, and what a refresh throws away.
#[cfg(test)]
mod gpui_tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    use gpui::{Entity, TestAppContext, VisualTestContext};
    use tty7_core::core::github::{ItemState, Reply, Transport};

    use super::*;
    use crate::ui::app::test_window;
    use crate::ui::github::Connection;

    #[derive(Default)]
    struct Fake {
        asked: Mutex<Vec<String>>,
        lists_fail: bool,
        /// `/issues/N` waits while this is set.
        hold: AtomicBool,
    }

    impl Transport for Fake {
        fn get(&self, path: &str) -> Result<Reply, ApiError> {
            self.asked.lock().unwrap().push(path.to_string());
            let body = if path.starts_with("/repos/acme/widgets/issues?") {
                if self.lists_fail {
                    return Err(ApiError::RateLimited { reset: None });
                }
                "[]".to_string()
            } else if path.starts_with("/repos/acme/widgets/pulls?") {
                r#"[{"number": 5, "title": "Merged since", "state": "closed",
                     "merged_at": "2026-09-03T10:00:00Z", "draft": false,
                     "created_at": "2026-09-01T10:00:00Z", "updated_at": "2026-09-03T10:00:00Z",
                     "html_url": "https://github.com/acme/widgets/pull/5",
                     "head": {"ref": "x"}, "base": {"ref": "main"}}]"#
                    .to_string()
            } else if let Some(n) = path.strip_prefix("/repos/acme/widgets/issues/") {
                while self.hold.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                format!(
                    r#"{{"number": {n}, "title": "looked up", "state": "open",
                         "created_at": "2026-09-01T10:00:00Z", "updated_at": "2026-09-01T10:00:00Z",
                         "html_url": "x"}}"#
                )
            } else {
                return Err(ApiError::NotFound);
            };
            Ok(Reply {
                body: body.into_bytes(),
                has_next: false,
            })
        }

        fn authenticated(&self) -> bool {
            true
        }
    }

    fn slug() -> RepoSlug {
        RepoSlug {
            owner: "acme".into(),
            name: "widgets".into(),
        }
    }

    fn setup(cx: &mut TestAppContext, fake: Arc<Fake>) -> (Entity<Tty7App>, VisualTestContext) {
        let (app, mut vcx) = test_window::harness(cx);
        let transport: Arc<dyn Transport> = fake;
        app.update_in(&mut vcx, |app, _, _| {
            app.github.connector = Some(Arc::new(move || Connection {
                transport: transport.clone(),
            }));
        });
        (app, vcx)
    }

    /// What one Session render does to the network, `times` over.
    fn passes(app: &Entity<Tty7App>, vcx: &mut VisualTestContext, numbers: &[u64], times: usize) {
        for _ in 0..times {
            app.update_in(vcx, |app, _, cx| {
                let slug = slug();
                app.github_session_lists(&slug, cx);
                let known = app.github_known(&slug);
                let missing = app.github_session_missing(&slug, numbers, &known);
                app.github_look_up(slug, missing, cx);
            });
            vcx.background_executor.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn lookups(fake: &Fake) -> usize {
        fake.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.starts_with("/repos/acme/widgets/issues/"))
            .count()
    }

    #[gpui::test]
    fn a_failed_list_stops_the_lookups(cx: &mut TestAppContext) {
        let fake = Arc::new(Fake {
            lists_fail: true,
            ..Default::default()
        });
        let (app, mut vcx) = setup(cx, fake.clone());
        passes(&app, &mut vcx, &[9, 5], 20);
        assert!(
            fake.asked.lock().unwrap().len() >= 2,
            "the lists were asked"
        );
        assert_eq!(lookups(&fake), 0);
    }

    #[gpui::test]
    fn a_stale_lookup_does_not_hide_a_fresher_list_row(cx: &mut TestAppContext) {
        let fake = Arc::new(Fake::default());
        let (app, mut vcx) = setup(cx, fake.clone());
        let old = Item {
            number: 5,
            title: "looked up".into(),
            state: ItemState::Open,
            is_pr: true,
            author: String::new(),
            labels: Vec::new(),
            comments: 0,
            created_at: 0,
            updated_at: 1,
            html_url: String::new(),
        };
        app.update_in(&mut vcx, |app, _, _| {
            app.github.session.looked_up.insert((slug(), 5), Some(old));
        });
        // Until 9 is looked up, the lists have landed.
        for _ in 0..200 {
            passes(&app, &mut vcx, &[9, 5], 1);
            if lookups(&fake) > 0 {
                break;
            }
        }
        let state = app.update_in(&mut vcx, |app, _, _| app.github_known(&slug())[&5].state);
        assert_eq!(state, ItemState::Merged);
        assert_eq!(lookups(&fake), 1, "5 was not looked up again, 9 was");
    }

    #[gpui::test]
    fn refresh_drops_a_lookup_in_flight(cx: &mut TestAppContext) {
        let fake = Arc::new(Fake::default());
        fake.hold.store(true, Ordering::SeqCst);
        let (app, mut vcx) = setup(cx, fake.clone());
        for _ in 0..200 {
            passes(&app, &mut vcx, &[9, 10], 1);
            if lookups(&fake) > 0 {
                break;
            }
        }
        assert_eq!(lookups(&fake), 1, "the worker is holding on #9");
        app.update_in(&mut vcx, |app, _, _| mark_due(&mut app.github.session));
        fake.hold.store(false, Ordering::SeqCst);
        for _ in 0..20 {
            vcx.background_executor.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(lookups(&fake), 1, "it stopped before #10");
        let kept = app.update_in(&mut vcx, |app, _, _| {
            (
                app.github.session.looked_up.len(),
                app.github.session.looking_up.len(),
            )
        });
        assert_eq!(kept, (0, 0));
    }
}
