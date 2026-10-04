//! The review box under a pull request's conversation: comment on it, or
//! approve it.

use std::collections::HashMap;

use gpui::{AnyElement, Context, Entity, Window, prelude::*, px};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{Disableable as _, Sizable as _, h_flex, v_flex};
use tty7_core::core::github::review::{ReviewEvent, submit_review};
use tty7_core::core::github::{ApiError, Detail, RepoSlug};

use crate::ui::app::Tty7App;
use crate::ui::github::off_ui;
use crate::ui::i18n::{L10nKey, t};
use crate::ui::panel_github::describe_error;
use crate::ui::right_panel::TEXT_INSET;

/// Drafts per pull request, so stepping to another one and back keeps what
/// was typed.
#[derive(Default)]
pub(crate) struct Reviews {
    drafts: HashMap<(RepoSlug, u64), Draft>,
}

struct Draft {
    input: Entity<InputState>,
    submitting: bool,
    error: Option<ApiError>,
}

impl Tty7App {
    /// `None` for an issue, or when signed out: GitHub takes no anonymous
    /// reviews.
    pub(crate) fn github_review_box(
        &mut self,
        slug: &RepoSlug,
        number: u64,
        detail: &Detail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !detail.item.is_pr || !self.github_authenticated() {
            return None;
        }
        let key = (slug.clone(), number);
        let draft = self
            .github
            .reviews
            .drafts
            .entry(key)
            .or_insert_with(|| Draft {
                input: cx.new(|cx| {
                    InputState::new(window, cx)
                        .auto_grow(3, 12)
                        .placeholder(t(L10nKey::GitHubReviewPlaceholder))
                }),
                submitting: false,
                error: None,
            });
        let (input, submitting, error) =
            (draft.input.clone(), draft.submitting, draft.error.clone());
        let error = error.map(|e| {
            let (text, hint) = describe_error(&e, true);
            self.panel_empty(&text, hint.as_deref(), cx)
        });
        let button = |id: &'static str, label: L10nKey, event: ReviewEvent| {
            let slug = slug.clone();
            Button::new(id)
                .label(t(label))
                .xsmall()
                .disabled(submitting)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.github_submit_review(slug.clone(), number, event, cx)
                }))
        };
        Some(
            v_flex()
                .mt(px(16.))
                .px(px(TEXT_INSET))
                .gap(px(6.))
                .child(Input::new(&input).small())
                .child(
                    h_flex()
                        .gap(px(4.))
                        .justify_end()
                        .child(button(
                            "panel-github-review-comment",
                            L10nKey::GitHubReviewComment,
                            ReviewEvent::Comment,
                        ))
                        .child(
                            button(
                                "panel-github-review-approve",
                                L10nKey::GitHubReviewApprove,
                                ReviewEvent::Approve,
                            )
                            .primary(),
                        ),
                )
                .children(error)
                .into_any_element(),
        )
    }

    /// On success the draft goes — the next render starts an empty one — and
    /// the detail is marked due, so the new review shows up.
    fn github_submit_review(
        &mut self,
        slug: RepoSlug,
        number: u64,
        event: ReviewEvent,
        cx: &mut Context<Self>,
    ) {
        let key = (slug.clone(), number);
        let Some(connection) = self.github.connection.clone() else {
            return;
        };
        let Some(draft) = self.github.reviews.drafts.get_mut(&key) else {
            return;
        };
        let body = draft.input.read(cx).value().trim().to_string();
        if event == ReviewEvent::Comment && body.is_empty() {
            return;
        }
        draft.submitting = true;
        draft.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let transport = connection.transport.clone();
            let Some(result) =
                off_ui(move || submit_review(&*transport, &slug, number, event, &body)).await
            else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.github.reviews.drafts.remove(&key);
                        if let Some(entry) = this.github.details.get_mut(&key) {
                            entry.fetched = None;
                        }
                    }
                    Err(e) => {
                        log::warn!("github: reviewing {}#{number}: {e}", key.0.full());
                        if let Some(draft) = this.github.reviews.drafts.get_mut(&key) {
                            draft.submitting = false;
                            draft.error = Some(e);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
