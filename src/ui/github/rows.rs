//! The list rows' stack connectors and hover links.

use gpui::{AnyElement, Div, Hsla, SharedString, div, prelude::*, px, rems};
use gpui_component::button::Button;
use gpui_component::{ActiveTheme as _, Icon, h_flex};

use tty7_core::core::github::stack::{StackShape, graphite_url};
use tty7_core::core::github::{Item, RepoSlug};

use crate::ui::i18n::{L10nKey, t};
use crate::ui::panel_github::{GLYPH, ROW_H, github_tile, state_glyph};
use crate::ui::right_panel::{META, ROW_INSET};

/// A stack's line down lane `lane` of the glyph column.
fn connector(lane: usize, h: f32, color: Hsla) -> Div {
    div()
        .absolute()
        .left(px(lane as f32 * GLYPH + GLYPH / 2. - 0.5))
        .w(px(1.))
        .h(px(h))
        .bg(color)
}

/// The state glyph in its lane, with a stacked pull request's connectors:
/// up and down its own lane, the side lanes passing it, and those ending in
/// it from a fork.
pub(crate) fn stack_glyph(item: &Item, shape: &StackShape, cx: &gpui::App) -> AnyElement {
    let color = cx.theme().muted_foreground.opacity(0.5);
    let gap = (ROW_H - GLYPH) / 2.;
    let x = |lane: usize| lane as f32 * GLYPH + GLYPH / 2.;
    let through = shape
        .through
        .iter()
        .map(|&l| connector(l, ROW_H, color).top_0());
    // A joining lane comes down half the gap, then across to the glyph's.
    let joins = shape.joins.iter().flat_map(|&l| {
        [
            connector(l, gap / 2., color).top_0(),
            div()
                .absolute()
                .top(px(gap / 2.))
                .left(px(x(shape.lane) - 0.5))
                .w(px(x(l) - x(shape.lane) + 1.))
                .h(px(1.))
                .bg(color),
        ]
    });
    let joined = !shape.joins.is_empty();
    div()
        .flex_none()
        .relative()
        .w(px(shape.lanes.max(1) as f32 * GLYPH))
        .h(px(ROW_H))
        .child(
            div()
                .absolute()
                .top(px(gap))
                .left(px(shape.lane as f32 * GLYPH))
                .child(state_glyph(item.state, item.is_pr, cx)),
        )
        .when(shape.up, |d| {
            d.child(connector(shape.lane, gap, color).top_0())
        })
        .when(joined && !shape.up, |d| {
            d.child(connector(shape.lane, gap / 2., color).top(px(gap / 2.)))
        })
        .when(shape.down, |d| {
            d.child(connector(shape.lane, gap, color).bottom_0())
        })
        .children(through)
        .children(joins)
        .into_any_element()
}

/// The branch a stack is based on, under its bottom row: the connector ends
/// in a dot beside the branch's name.
pub(crate) fn stack_trunk(base: &str, cx: &gpui::App) -> AnyElement {
    let theme = cx.theme();
    let color = theme.muted_foreground.opacity(0.5);
    const H: f32 = 18.;
    const DOT: f32 = 5.;
    h_flex()
        .h(px(H))
        .px(px(ROW_INSET))
        .gap(px(8.))
        .items_center()
        .child(
            div()
                .flex_none()
                .relative()
                .w(px(GLYPH))
                .h(px(H))
                .child(connector(0, H / 2., color).top_0())
                .child(
                    div()
                        .absolute()
                        .top(px((H - DOT) / 2.))
                        .left(px((GLYPH - DOT) / 2.))
                        .size(px(DOT))
                        .rounded_full()
                        .bg(color),
                ),
        )
        .child(
            div()
                .text_size(rems(META))
                .font_family(theme.mono_font_family.clone())
                .text_color(theme.muted_foreground)
                .child(base.to_string()),
        )
        .into_any_element()
}

/// A hovered row's way out to GitHub, and for a pull request to Graphite.
pub(crate) fn hovered_links(slug: &RepoSlug, item: &Item, cx: &gpui::App) -> AnyElement {
    let number = item.number;
    let github = link_tile(
        SharedString::from(format!("panel-github-row-gh-{number}")),
        Icon::empty().path("icons/github.svg"),
        t(L10nKey::GitHubOpenOnGitHub),
        item.html_url.clone(),
        cx,
    );
    let graphite = item.is_pr.then(|| {
        link_tile(
            SharedString::from(format!("panel-github-row-gt-{number}")),
            Icon::empty().path("icons/graphite.svg"),
            t(L10nKey::GitHubOpenInGraphite),
            graphite_url(&slug.owner, &slug.name, number),
            cx,
        )
    });
    h_flex()
        .flex_none()
        .gap(px(2.))
        .child(github)
        .children(graphite)
        .into_any_element()
}

/// A tile that opens `url`. The click stops at the tile, so the row does not
/// open its detail too.
fn link_tile(
    id: SharedString,
    icon: Icon,
    tooltip: &'static str,
    url: String,
    cx: &gpui::App,
) -> Button {
    github_tile(id, icon, tooltip, cx)
        .cursor_pointer()
        .on_click(move |_, _window, cx| {
            cx.stop_propagation();
            cx.open_url(&url);
        })
}
