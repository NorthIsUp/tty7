//! The fork's sidebar group header: its outline and fill, its chevron, the
//! repo default branch it names, and the Settings rows for all of it. Kept out
//! of `tab_sidebar.rs` for the reason `group_color.rs` is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::{AnyElement, App, Context, Global, Hsla, IntoElement, Styled};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::core::config::{Config, GroupColorSource};
use crate::terminal::git_status::GitStatusCache;
use crate::ui::app::Tty7App;
use crate::ui::group_color::{active_ansi16, group_color};
use crate::ui::host_ops::{HostId, HostOps};
use crate::ui::host_registry::HostRegistry;
use crate::ui::i18n::{L10nKey, t};

/// A hashed fill's alpha: a pastel over a light rail, a wash over a dark one,
/// and the muted name stays readable on both.
const FILL_ALPHA: f32 = 0.14;
/// A hashed outline's alpha. At full strength a 1px ring of every hue in the
/// palette is the loudest thing in the column.
const OUTLINE_ALPHA: f32 = 0.6;

pub(crate) struct HeaderStyle {
    fill: Option<Hsla>,
    outline: Option<Hsla>,
    /// What the header's hover buttons paint under themselves so they cover
    /// the branch: the rail with this header's fill laid over it.
    pub(crate) backing: Hsla,
}

/// `Theme` answers with `theme`, `Hashed` with the group's own colour — its
/// override when it has one.
pub(crate) fn source_color(
    source: GroupColorSource,
    name: &str,
    overrides: &HashMap<String, String>,
    ansi16: &[(u8, u8, u8); 16],
    theme: Hsla,
) -> Hsla {
    match source {
        GroupColorSource::Theme => theme,
        GroupColorSource::Hashed => group_color(name, overrides, ansi16),
    }
}

pub(crate) fn header_style(name: &str, rail: Hsla, cx: &App) -> HeaderStyle {
    let cfg = cx.global::<Config>();
    let ansi16 = active_ansi16(cx);
    let theme = cx.theme();
    let color = |source, neutral| source_color(source, name, &cfg.group_colors, &ansi16, neutral);
    let fill = cfg
        .group_background
        .then(|| color(cfg.group_background_color, theme.muted_foreground).opacity(FILL_ALPHA));
    let outline = cfg.group_outline.then(|| match cfg.group_outline_color {
        GroupColorSource::Theme => theme.border,
        GroupColorSource::Hashed => {
            color(GroupColorSource::Hashed, theme.border).opacity(OUTLINE_ALPHA)
        }
    });
    HeaderStyle {
        fill,
        outline,
        backing: fill.map_or(rail, |f| rail.blend(f)),
    }
}

pub(crate) fn decorate<E: Styled>(el: E, style: &HeaderStyle) -> E {
    let el = match style.fill.is_some() || style.outline.is_some() {
        true => el.rounded(crate::ui::rounding::ROW_RADIUS),
        false => el,
    };
    let el = match style.fill {
        Some(c) => el.bg(c),
        None => el,
    };
    match style.outline {
        Some(c) => el.border_1().border_color(c),
        None => el,
    }
}

pub(crate) fn chevron(folded: bool) -> Icon {
    Icon::new(match folded {
        true => IconName::ChevronRight,
        false => IconName::ChevronDown,
    })
    .xsmall()
}

/// Every repo's default branch this process has looked up, by host and
/// repo home, so the linked worktrees of one repo share one answer. `None` is
/// both "asked, still waiting" and "asked, there is none".
// ponytail: never refreshed; a changed origin/HEAD shows after a restart.
#[derive(Default)]
struct DefaultBranches(HashMap<(HostId, PathBuf), Option<String>>);

impl Global for DefaultBranches {}

/// The default branch of the repo `cwd` is in, once known. The first ask
/// starts the lookup off the UI thread and repaints when it lands.
pub(crate) fn default_branch(
    host: HostId,
    cwd: &Path,
    cx: &mut Context<Tty7App>,
) -> Option<String> {
    let home = cx
        .try_global::<GitStatusCache>()?
        .known_repo_for(host, cwd)??;
    let key = (host, home);
    let known = &mut cx.default_global::<DefaultBranches>().0;
    if let Some(branch) = known.get(&key) {
        return branch.clone();
    }
    known.insert(key.clone(), None);
    let shared = HostRegistry::lookup(cx, host)?;
    let dir = key.1.clone();
    HostOps::run(
        shared,
        cx,
        move |h| {
            let git = |args: &[&str]| tty7_core::core::git::git(h, &dir, args);
            default_branch_name(
                git(&["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"]).as_deref(),
                || git(&["config", "--get", "init.defaultBranch"]),
            )
        },
        move |_, branch, cx| {
            if branch.is_some() {
                cx.global_mut::<DefaultBranches>().0.insert(key, branch);
                cx.notify();
            }
        },
    );
    None
}

/// `refs/remotes/origin/main` → `main`; without one, `init.defaultBranch`.
fn default_branch_name(
    origin_head: Option<&str>,
    init_default: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let nonempty = |s: &str| Some(s.trim()).filter(|s| !s.is_empty()).map(str::to_string);
    origin_head
        .and_then(|r| r.trim().strip_prefix("refs/remotes/origin/"))
        .and_then(nonempty)
        .or_else(|| init_default().as_deref().and_then(nonempty))
}

impl Tty7App {
    /// The four header options, as rows for Settings' Tabs group.
    pub(crate) fn group_header_settings(&self, cx: &mut Context<Self>) -> [AnyElement; 4] {
        let cfg = cx.global::<Config>();
        let (outline, fill) = (cfg.group_outline, cfg.group_background);
        let ix = |s: GroupColorSource| match s {
            GroupColorSource::Theme => 0,
            GroupColorSource::Hashed => 1,
        };
        let (outline_ix, fill_ix) = (ix(cfg.group_outline_color), ix(cfg.group_background_color));
        let pick = |ix: usize| match ix {
            0 => GroupColorSource::Theme,
            _ => GroupColorSource::Hashed,
        };
        let sources = [
            t(L10nKey::SettingsGroupColorTheme),
            t(L10nKey::SettingsGroupColorHashed),
        ];
        let controls = [
            self.settings_switch("wt-group-outline", outline, cx, |this, on, _, cx| {
                this.update_config(cx, |c| c.group_outline = on)
            }),
            self.settings_choice(
                "wt-group-outline-color",
                &sources,
                outline_ix,
                cx,
                move |this, ix, _, cx| this.update_config(cx, |c| c.group_outline_color = pick(ix)),
            ),
            self.settings_switch("wt-group-background", fill, cx, |this, on, _, cx| {
                this.update_config(cx, |c| c.group_background = on)
            }),
            self.settings_choice(
                "wt-group-background-color",
                &sources,
                fill_ix,
                cx,
                move |this, ix, _, cx| {
                    this.update_config(cx, |c| c.group_background_color = pick(ix))
                },
            ),
        ];
        let labels = [
            (
                L10nKey::SettingsGroupOutline,
                L10nKey::SettingsGroupOutlineDesc,
            ),
            (
                L10nKey::SettingsGroupOutlineColor,
                L10nKey::SettingsGroupOutlineColorDesc,
            ),
            (
                L10nKey::SettingsGroupBackground,
                L10nKey::SettingsGroupBackgroundDesc,
            ),
            (
                L10nKey::SettingsGroupBackgroundColor,
                L10nKey::SettingsGroupBackgroundColorDesc,
            ),
        ];
        let mut controls = controls.into_iter();
        labels.map(|(label, desc)| {
            self.settings_row(t(label), t(desc), controls.next().unwrap(), cx)
                .into_any_element()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSI16: [(u8, u8, u8); 16] = [(10, 20, 30); 16];

    #[test]
    fn theme_ignores_the_group_and_hashed_is_its_colour_with_the_override_winning() {
        let theme: Hsla = gpui::rgb(0x123456).into();
        let none = HashMap::new();
        let over = HashMap::from([("tty7".to_string(), "#ff8800".to_string())]);
        assert_eq!(
            source_color(GroupColorSource::Theme, "tty7", &over, &ANSI16, theme),
            theme
        );
        assert_eq!(
            source_color(GroupColorSource::Hashed, "tty7", &none, &ANSI16, theme),
            group_color("tty7", &none, &ANSI16)
        );
        assert_eq!(
            source_color(GroupColorSource::Hashed, "tty7", &over, &ANSI16, theme),
            Hsla::from(gpui::rgb(0xff8800))
        );
    }

    #[test]
    fn default_branch_is_origin_head_else_init_default_else_none() {
        let init = || Some("trunk\n".to_string());
        let unset = || None;
        assert_eq!(
            default_branch_name(Some("refs/remotes/origin/main\n"), init).as_deref(),
            Some("main")
        );
        assert_eq!(
            default_branch_name(Some("refs/remotes/origin/release/2.x"), unset).as_deref(),
            Some("release/2.x")
        );
        assert_eq!(default_branch_name(None, init).as_deref(), Some("trunk"));
        assert_eq!(
            default_branch_name(Some("refs/remotes/origin/"), init).as_deref(),
            Some("trunk")
        );
        assert_eq!(default_branch_name(None, unset), None);
        assert_eq!(default_branch_name(None, || Some("  ".into())), None);
    }
}
