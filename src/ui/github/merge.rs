//! The merge buttons under an open pull request's merge state: one per
//! method the repository allows, and auto-merge on or off.

use std::collections::HashMap;

use gpui::{AnyElement, Context, PromptButton, PromptLevel, Window, prelude::*, px};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{Disableable as _, Sizable as _, h_flex, v_flex};
use tty7_core::core::github::merge::{MergeMethod, MergeRules, merge, merge_rules, set_auto_merge};
use tty7_core::core::github::{ApiError, Detail, ItemState, RepoSlug};

use crate::ui::app::Tty7App;
use crate::ui::github::off_ui;
use crate::ui::i18n::{L10nKey, t, t_fmt};
use crate::ui::panel_github::describe_error;
use crate::ui::right_panel::TEXT_INSET;

#[derive(Default)]
pub(crate) struct Merges {
    /// Read once per repository; `None` while the read is in flight.
    rules: HashMap<RepoSlug, Option<MergeRules>>,
    /// Pull requests with a merge in flight (`None`) or one that failed.
    pending: HashMap<(RepoSlug, u64), Option<ApiError>>,
}

#[derive(Clone, Copy)]
enum Action {
    Merge(MergeMethod),
    /// `None` turns it off.
    AutoMerge(Option<MergeMethod>),
}

impl Action {
    fn id(self) -> &'static str {
        match self {
            Action::Merge(MergeMethod::Merge) => "panel-github-merge-commit",
            Action::Merge(MergeMethod::Squash) => "panel-github-merge-squash",
            Action::Merge(MergeMethod::Rebase) => "panel-github-merge-rebase",
            Action::AutoMerge(Some(_)) => "panel-github-auto-merge-on",
            Action::AutoMerge(None) => "panel-github-auto-merge-off",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Action::Merge(m) => method_label(m),
            Action::AutoMerge(Some(_)) => t(L10nKey::GitHubAutoMergeEnable),
            Action::AutoMerge(None) => t(L10nKey::GitHubAutoMergeDisable),
        }
    }
}

fn method_label(m: MergeMethod) -> &'static str {
    t(match m {
        MergeMethod::Merge => L10nKey::GitHubMergeCommit,
        MergeMethod::Squash => L10nKey::GitHubMergeSquash,
        MergeMethod::Rebase => L10nKey::GitHubMergeRebase,
    })
}

impl Tty7App {
    /// `None` unless the pull request is open, not a draft, and the session
    /// is signed in — and until the repository's rules are read.
    pub(crate) fn github_merge_box(
        &mut self,
        slug: &RepoSlug,
        number: u64,
        detail: &Detail,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let pull = detail.pull.as_ref()?;
        if detail.item.state != ItemState::Open || !self.github_authenticated() {
            return None;
        }
        self.github_ensure_merge_rules(slug, cx);
        let rules = self.github.merges.rules.get(slug).cloned().flatten()?;
        let (busy, error) = match self.github.merges.pending.get(&(slug.clone(), number)) {
            Some(error) => (error.is_none(), error.clone()),
            None => (false, None),
        };
        let mut actions: Vec<Action> = rules.methods.iter().map(|&m| Action::Merge(m)).collect();
        if !pull.node_id.is_empty() {
            if pull.auto_merge {
                actions.push(Action::AutoMerge(None));
            } else if rules.auto_merge
                && let Some(&m) = rules.methods.first()
            {
                actions.push(Action::AutoMerge(Some(m)));
            }
        }
        let error = error.map(|e| {
            let (text, hint) = describe_error(&e, true);
            self.panel_empty(&text, hint.as_deref(), cx)
        });
        let buttons = actions.into_iter().enumerate().map(|(i, action)| {
            let slug = slug.clone();
            let button = Button::new(action.id())
                .label(action.label())
                .xsmall()
                .disabled(busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.github_confirm_merge(slug.clone(), number, action, window, cx)
                }));
            if i == 0 { button.primary() } else { button }
        });
        Some(
            v_flex()
                .mt(px(8.))
                .px(px(TEXT_INSET))
                .gap(px(6.))
                .child(h_flex().flex_wrap().gap(px(4.)).children(buttons))
                .children(error)
                .into_any_element(),
        )
    }

    fn github_ensure_merge_rules(&mut self, slug: &RepoSlug, cx: &mut Context<Self>) {
        if self.github.merges.rules.contains_key(slug) {
            return;
        }
        let Some(connection) = self.github.connection.clone() else {
            return;
        };
        self.github.merges.rules.insert(slug.clone(), None);
        let slug = slug.clone();
        cx.spawn(async move |this, cx| {
            let (transport, read) = (connection.transport.clone(), slug.clone());
            let Some(result) = off_ui(move || merge_rules(&*transport, &read)).await else {
                return;
            };
            // Unreadable rules are the unknown ones: every method offered,
            // and GitHub says no if it must.
            let rules = result.unwrap_or_else(|e| {
                log::warn!("github: merge rules of {}: {e}", slug.full());
                MergeRules::default()
            });
            let _ = this.update(cx, |this, cx| {
                this.github.merges.rules.insert(slug, Some(rules));
                cx.notify();
            });
        })
        .detach();
    }

    /// A merge cannot be taken back, and auto-merge is one waiting to
    /// happen, so both ask first; turning auto-merge off does not.
    fn github_confirm_merge(
        &mut self,
        slug: RepoSlug,
        number: u64,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pull) = self
            .github
            .details
            .get(&(slug.clone(), number))
            .and_then(|e| e.detail.as_ref()?.pull.clone())
        else {
            return;
        };
        let (base, node_id) = (pull.base_ref.as_str(), pull.node_id.clone());
        let number_text = number.to_string();
        let question = match action {
            Action::AutoMerge(None) => {
                return self.github_run_merge(slug, number, node_id, action, cx);
            }
            Action::Merge(_) => t_fmt(
                L10nKey::GitHubMergeConfirm,
                &[("number", &number_text), ("base", base)],
            ),
            Action::AutoMerge(Some(m)) => t_fmt(
                L10nKey::GitHubAutoMergeConfirm,
                &[
                    ("number", &number_text),
                    ("base", base),
                    ("method", method_label(m)),
                ],
            ),
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &question,
            None,
            &[
                PromptButton::cancel(t(L10nKey::Cancel)),
                PromptButton::new(action.label()),
            ],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(1) = answer.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.github_run_merge(slug, number, node_id, action, cx)
            });
        })
        .detach();
    }

    /// Success marks the detail due, so the merged state — or auto-merge's —
    /// shows up.
    fn github_run_merge(
        &mut self,
        slug: RepoSlug,
        number: u64,
        node_id: String,
        action: Action,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.github.connection.clone() else {
            return;
        };
        let key = (slug.clone(), number);
        self.github.merges.pending.insert(key.clone(), None);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let transport = connection.transport.clone();
            let Some(result) = off_ui(move || match action {
                Action::Merge(m) => merge(&*transport, &slug, number, m),
                Action::AutoMerge(m) => set_auto_merge(&*transport, &node_id, m),
            })
            .await
            else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.github.merges.pending.remove(&key);
                        if let Some(entry) = this.github.details.get_mut(&key) {
                            entry.fetched = None;
                        }
                    }
                    Err(e) => {
                        log::warn!("github: merging {}#{number}: {e}", key.0.full());
                        this.github.merges.pending.insert(key, Some(e));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
