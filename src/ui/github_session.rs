//! The GitHub tab's "This session" filter: its Pull Requests narrowed to the
//! ones the focused pane's agent session mentions
//! ([`ForkHost::agent_session_mentions`](tty7_core::core::fork_host::ForkHost::agent_session_mentions)).
//!
//! The list is the fetched pages intersected with the mentions. A mentioned
//! pull request that is not on them (merged, closed, older than the page)
//! gets a `#N` chip in a "more mentioned" row that opens its detail, so each
//! costs requests only when clicked.

use std::sync::Arc;
use std::time::Instant;

use gpui::{AnyElement, Context, SharedString, Window, div, prelude::*, px, rems};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use tty7_core::core::cli_agent::CLIAgent;
use tty7_core::core::fork_host::ForkCalls as _;
use tty7_core::core::github::{Item, Kind, RepoSlug};
use tty7_core::core::history_search::Mentions;
use tty7_core::host::HostId;

use crate::core::config::Config;
use crate::ui::app::{CONTENT_INSET, Tty7App};
use crate::ui::github::{GitHubPanelState, STALE_AFTER};
use crate::ui::host_ops::{HostOps, SharedHost};
use crate::ui::i18n::{L10nKey, t, t_fmt};
use crate::ui::right_panel::{META, TEXT_INSET};
use tty7_core::core::fork_config::GitHubPanelList;

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

/// Refresh reads the session again.
pub(crate) fn mark_due(cache: &mut Option<SessionCache>) {
    if let Some(c) = cache {
        c.fetched = None;
    }
}

/// The fetched rows the session mentions, and the mentioned pull requests
/// that are not among them.
pub(crate) fn intersect(items: &[Item], m: &Mentions) -> (Vec<Item>, Vec<u64>) {
    let shown: Vec<Item> = items
        .iter()
        .filter(|i| m.refs.contains(&i.number) || m.issue_links.contains(&i.number))
        .cloned()
        .collect();
    let more = m
        .refs
        .iter()
        .copied()
        .filter(|n| !shown.iter().any(|i| i.number == *n))
        .collect();
    (shown, more)
}

/// The remote to prefer: the user's pick, else `origin` (the fork) when
/// `prefer_origin`, else upstream's own order in `default_remote`.
pub(crate) fn remote_pick(pick: Option<&str>, prefer_origin: bool) -> Option<&str> {
    pick.or(prefer_origin.then_some("origin"))
}

/// The panel's state at launch: the list `github_panel_default_list` names.
pub(crate) fn panel_state(config: &Config) -> GitHubPanelState {
    GitHubPanelState {
        kind: match config.fork.github_panel_default_list {
            GitHubPanelList::Issues => Kind::Issues,
            GitHubPanelList::PullRequests => Kind::Pulls,
        },
        ..Default::default()
    }
}

impl Tty7App {
    fn github_session_on(&self, cx: &gpui::App) -> bool {
        self.github.kind == Kind::Pulls
            && cx
                .global::<crate::core::config::Config>()
                .fork
                .github_panel_session_filter
    }

    /// The chip beside Open / Closed, on Pull Requests only.
    pub(crate) fn github_session_chip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.github.kind != Kind::Pulls {
            return None;
        }
        let on = self.github_session_on(cx);
        Some(
            crate::ui::panel_github::switch_cell(
                ("panel-github-session", 0),
                t(L10nKey::GitHubThisSession),
                on,
                cx,
            )
            .ml(px(6.))
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.github_set_session_filter(!on, cx);
            }))
            .into_any_element(),
        )
    }

    fn github_set_session_filter(&mut self, on: bool, cx: &mut Context<Self>) {
        self.update_config(cx, |c| c.fork.github_panel_session_filter = on);
        self.github.list_scroll = gpui::ScrollHandle::new();
        cx.notify();
    }

    /// The filtered list body, or `None` when the filter is off or the list
    /// itself has nothing to filter yet (loading, failed), which the plain
    /// body says better.
    pub(crate) fn github_session_body(
        &mut self,
        host: &SharedHost,
        slug: &RepoSlug,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.github_session_on(cx) {
            return None;
        }
        let cache = self.github.lists.get(&self.github_query(slug))?;
        if !cache.loaded || cache.error.is_some() {
            return None;
        }
        let items = cache.items.clone();
        let mentions = match self.github_session_key(host, slug, window, cx) {
            None => Some(Arc::new(Mentions::default())),
            Some(key) => self.github_ensure_mentions(host, key, cx),
        };
        let Some(mentions) = mentions else {
            return Some(self.panel_empty(t(L10nKey::PanelLoading), None, cx));
        };
        let (shown, more) = intersect(&items, &mentions);
        if shown.is_empty() && more.is_empty() {
            return Some(self.github_session_none(cx));
        }
        let now = crate::ui::github::now_unix();
        let mut rows = v_flex().px(px(CONTENT_INSET));
        for item in &shown {
            rows = rows.child(self.github_item_row(slug, item, now, cx));
        }
        Some(
            v_flex()
                .pb(px(12.))
                .child(rows)
                .children((!more.is_empty()).then(|| self.github_session_more(slug, &more, cx)))
                .into_any_element(),
        )
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
        let same = self.github.session.as_ref().is_some_and(|c| c.key == key);
        if !same {
            self.github.session = Some(SessionCache {
                key: key.clone(),
                mentions: None,
                loading: false,
                fetched: None,
            });
        }
        let cache = self.github.session.as_mut()?;
        if !cache.loading && cache.fetched.is_none_or(|t| t.elapsed() > STALE_AFTER) {
            cache.loading = true;
            let asked = key.clone();
            HostOps::run(
                host.clone(),
                cx,
                move |h| {
                    h.agent_session_mentions(asked.agent, &asked.id, &asked.slug)
                        .unwrap_or_default()
                },
                move |this, mentions, cx| {
                    let Some(c) = this.github.session.as_mut().filter(|c| c.key == key) else {
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

    /// Nothing to show: say so, with the way back to the whole list.
    fn github_session_none(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(TEXT_INSET))
            .py(px(4.))
            .text_size(rems(crate::ui::right_panel::TEXT))
            .text_color(cx.theme().muted_foreground)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(t(L10nKey::GitHubNoSessionPulls)),
            )
            .child(
                Button::new("panel-github-session-off")
                    .ghost()
                    .xsmall()
                    .label(t(L10nKey::GitHubShowAllPulls))
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.github_set_session_filter(false, cx);
                    })),
            )
            .into_any_element()
    }

    /// "N more mentioned", then a chip per number that opens its detail.
    fn github_session_more(
        &self,
        slug: &RepoSlug,
        more: &[u64],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let sf = cx.global::<crate::ui::presets::Surfaces>().sidebar;
        let mono = cx.theme().mono_font_family.clone();
        let chips = more.iter().map(|&number| {
            let slug = slug.clone();
            div()
                .id(SharedString::from(format!(
                    "panel-github-mentioned-{number}"
                )))
                .px(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .font_family(mono.clone())
                .hover(|s| s.bg(gpui::rgb(sf.hover)))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.github_open_detail(slug.clone(), number, cx);
                }))
                .child(format!("#{number}"))
        });
        v_flex()
            .gap(px(2.))
            .px(px(TEXT_INSET))
            .pt(px(8.))
            .text_size(rems(META))
            .text_color(muted)
            .child(t_fmt(
                L10nKey::GitHubMoreMentioned,
                &[("count", &more.len().to_string())],
            ))
            .child(h_flex().flex_wrap().gap(px(2.)).ml(px(-4.)).children(chips))
            .into_any_element()
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
    fn the_panel_opens_on_the_configured_list() {
        let mut config = Config::default();
        assert_eq!(panel_state(&config).kind, Kind::Issues);
        config.fork.github_panel_default_list = GitHubPanelList::PullRequests;
        assert_eq!(panel_state(&config).kind, Kind::Pulls);
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
    fn the_filter_keeps_mentioned_rows_and_names_the_rest() {
        let items: Vec<Item> = [5, 4, 3, 2].map(item).to_vec();
        let m = Mentions {
            refs: vec![9, 4, 2],
            issue_links: vec![3, 8],
        };
        let (shown, more) = intersect(&items, &m);
        let numbers: Vec<u64> = shown.iter().map(|i| i.number).collect();
        assert_eq!(numbers, vec![4, 3, 2], "the list's order, issue links too");
        assert_eq!(more, vec![9], "an issue link off the page may be an issue");

        let (shown, more) = intersect(&items, &Mentions::default());
        assert!(shown.is_empty() && more.is_empty());
    }
}
