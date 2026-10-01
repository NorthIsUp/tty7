//! The fork's sidebar group header: its outline and fill, its chevron, the
//! repo default branch it names, and the Settings rows for all of it. Kept out
//! of `tab_sidebar.rs` for the reason `group_color.rs` is.

use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Context, EntityId, Global, Hsla, IntoElement, ParentElement as _, PromptLevel,
    Styled, Window, ease_in_out, px, radians,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};

use crate::core::config::Config;
use crate::core::group_key::{GroupId, GroupKey};
use crate::terminal::git_status::{GitStatus, GitStatusCache};
use crate::ui::app::Tty7App;
use crate::ui::group_color::{dark_rail, group_color};
use crate::ui::host_ops::{HostId, HostOps};
use crate::ui::host_registry::HostRegistry;
use crate::ui::i18n::{L10nKey, t, t_fmt};
use crate::ui::tab_sidebar::SharedGit;
use tty7_core::core::fork_config::{GroupBackgroundScope, GroupColorSource};

/// A hashed fill's alpha: a pastel over a light rail, a wash over a dark one,
/// and the muted name stays readable on both.
pub(crate) const FILL_ALPHA: f32 = 0.14;
/// A hashed outline's alpha. At full strength a 1px ring of every hue in the
/// palette is the loudest thing in the column.
const OUTLINE_ALPHA: f32 = 0.6;

/// How long a group takes to fold open or shut: an `NSOutlineView`'s pace.
const FOLD: Duration = Duration::from_millis(200);

pub(crate) struct HeaderStyle {
    scope: GroupBackgroundScope,
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
    slot: Option<usize>,
    overrides: &HashMap<String, String>,
    dark: bool,
    theme: Hsla,
) -> Hsla {
    match source {
        GroupColorSource::Theme => theme,
        GroupColorSource::Hashed => group_color(name, slot, overrides, dark),
    }
}

/// `slot` is the group's place in the sidebar, `None` for Ungrouped.
fn header_style(name: &str, slot: Option<usize>, rail: Hsla, cx: &App) -> HeaderStyle {
    let cfg = cx.global::<Config>();
    let dark = dark_rail(cx);
    let theme = cx.theme();
    let color =
        |source, neutral| source_color(source, name, slot, &cfg.fork.group_colors, dark, neutral);
    let fill = cfg.fork.group_background.then(|| {
        color(cfg.fork.group_background_color, theme.muted_foreground).opacity(FILL_ALPHA)
    });
    let outline = cfg
        .fork
        .group_outline
        .then(|| match cfg.fork.group_outline_color {
            GroupColorSource::Theme => theme.border,
            GroupColorSource::Hashed => {
                color(GroupColorSource::Hashed, theme.border).opacity(OUTLINE_ALPHA)
            }
        });
    HeaderStyle {
        scope: cfg.fork.group_background_scope,
        fill,
        outline,
        backing: fill.map_or(rail, |f| rail.blend(f)),
    }
}

/// Whether the header bar (`block == false`) or the block holding the header
/// and its rows wears the fill and outline under `scope`.
fn wears(scope: GroupBackgroundScope, block: bool) -> bool {
    match scope {
        GroupBackgroundScope::Header => !block,
        GroupBackgroundScope::Group => block,
    }
}

/// One sidebar group's decoration for one frame: its colours, its fill and
/// outline, and how far open it is drawn while it folds. Worked out once per
/// group, so the render loop only asks it for each piece.
pub(crate) struct Deco {
    /// `None` for the one section without a header (Ungrouped with nothing
    /// above it), which gets no decoration.
    style: Option<HeaderStyle>,
    swatch: Hsla,
    open: f32,
    rail: Hsla,
}

impl Deco {
    /// `ix` is the group's place in the sidebar; the hue follows it for every
    /// group but Ungrouped (`key == None`).
    pub(crate) fn new(
        name: Option<&str>,
        key: Option<&GroupKey>,
        folded: bool,
        ix: usize,
        rail: Hsla,
        window: &Window,
        cx: &mut Context<Tty7App>,
    ) -> Self {
        let slot = key.is_some().then_some(ix);
        let open = match name {
            Some(_) => openness(key, folded, window, cx),
            None => 1.,
        };
        let cfg = cx.global::<Config>();
        let swatch = group_color(
            name.unwrap_or_default(),
            slot,
            &cfg.fork.group_colors,
            dark_rail(cx),
        );
        Deco {
            style: name.map(|n| header_style(n, slot, rail, cx)),
            swatch,
            open,
            rail,
        }
    }

    /// A folded group's rows are gone only once its slide has finished.
    pub(crate) fn folded_away(&self, folded: bool) -> bool {
        folded && self.open <= 0.
    }

    /// The header bar's fill, when the fill covers the header alone.
    pub(crate) fn bar<E: Styled>(&self, el: E) -> E {
        match &self.style {
            Some(style) => paint(el, style.fill.filter(|_| wears(style.scope, false)), None),
            None => el,
        }
    }

    /// The group block's outline, always: it marks where the group ends, so
    /// it goes round the rows too. Its fill, when the fill covers the group.
    pub(crate) fn block<E: Styled>(&self, el: E) -> E {
        match &self.style {
            Some(style) => paint(
                el,
                style.fill.filter(|_| wears(style.scope, true)),
                style.outline,
            ),
            None => el,
        }
    }

    /// ▸ shut, turning clockwise to ▾ open.
    pub(crate) fn chevron(&self) -> Icon {
        chevron(self.open)
    }

    /// The dot before the header's name.
    pub(crate) fn swatch(&self) -> impl IntoElement {
        gpui::div()
            .flex_shrink_0()
            .size(px(crate::ui::group_color::SWATCH))
            .rounded_full()
            .bg(self.swatch)
    }

    /// What the header's hover buttons paint under themselves so they cover
    /// the branch: the rail with the header's fill laid over it.
    pub(crate) fn backing(&self) -> Hsla {
        self.style.as_ref().map_or(self.rail, |s| s.backing)
    }

    /// The rows, clipped to how open the group is while it slides; `full` is
    /// their height open, `gap` the space between them.
    pub(crate) fn clip(&self, rows: Vec<AnyElement>, full: f32, gap: f32) -> Vec<AnyElement> {
        clip_rows(rows, self.open, full, gap)
    }
}

fn paint<E: Styled>(el: E, fill: Option<Hsla>, outline: Option<Hsla>) -> E {
    let el = match fill.is_some() || outline.is_some() {
        true => el.rounded(crate::ui::rounding::ROW_RADIUS),
        false => el,
    };
    let el = match fill {
        Some(c) => el.bg(c),
        None => el,
    };
    match outline {
        Some(c) => el.border_1().border_color(c),
        None => el,
    }
}

/// ▸ at `open == 0`, turning clockwise to ▾ at `open == 1`.
fn chevron(open: f32) -> Icon {
    Icon::new(IconName::ChevronRight)
        .xsmall()
        .rotate(radians(open * FRAC_PI_2))
}

struct Fold {
    folded: bool,
    since: Option<Instant>,
    drawn: Instant,
}

/// The last fold state each window's groups were drawn in, and when it last
/// changed, so any path that folds a group (a click, ^x^g, a search) slides.
#[derive(Default)]
struct Folds(HashMap<(EntityId, Option<GroupKey>), Fold>);

/// How long a fold state outlives its group's last draw: a deleted group's,
/// or a closed window's. One dropped by mistake only costs a slide, since it
/// comes back at rest.
const FOLD_FORGET: Duration = Duration::from_secs(60);

impl Global for Folds {}

/// How open group `key` is drawn this frame, 0 shut to 1 open. A change in
/// `folded` since the last frame starts the slide; while it runs the window
/// asks for another frame.
fn openness(
    key: Option<&GroupKey>,
    folded: bool,
    window: &Window,
    cx: &mut Context<Tty7App>,
) -> f32 {
    let animate = cx.global::<Config>().fork.animations;
    let app = cx.entity_id();
    let now = Instant::now();
    let folds = &mut cx.default_global::<Folds>().0;
    folds.retain(|_, f| now - f.drawn < FOLD_FORGET);
    let fold = folds.entry((app, key.cloned())).or_insert(Fold {
        folded,
        since: None,
        drawn: now,
    });
    fold.drawn = now;
    if fold.folded != folded {
        let in_flight = fold.since.map(|s| now - s);
        fold.since = animate.then(|| now - restart(in_flight));
        fold.folded = folded;
    }
    let elapsed = fold.since.map(|s| now - s);
    match elapsed.is_some_and(|e| e < FOLD) {
        true => window.request_animation_frame(),
        false => fold.since = None,
    }
    openness_at(folded, elapsed)
}

/// Eased openness `elapsed` into a slide towards `folded`; `None` is at rest.
fn openness_at(folded: bool, elapsed: Option<Duration>) -> f32 {
    let p = elapsed.map_or(1., |e| (e.as_secs_f32() / FOLD.as_secs_f32()).min(1.));
    match folded {
        true => 1. - ease_in_out(p),
        false => ease_in_out(p),
    }
}

/// Where a new slide starts. Reversing one mid-flight picks up at the mirror
/// point, which `ease_in_out`'s symmetry puts at the same openness.
fn restart(in_flight: Option<Duration>) -> Duration {
    in_flight
        .filter(|e| *e < FOLD)
        .map_or(Duration::ZERO, |e| FOLD - e)
}

/// A group's rows, clipped to `open` of their `full` height while they slide;
/// at rest they pass through untouched.
fn clip_rows(rows: Vec<AnyElement>, open: f32, full: f32, gap: f32) -> Vec<AnyElement> {
    if open >= 1. || rows.is_empty() {
        return rows;
    }
    vec![
        v_flex()
            .w_full()
            .flex_shrink_0()
            .gap(px(gap))
            .overflow_hidden()
            .h(px(full * open))
            .children(rows)
            .into_any_element(),
    ]
}

/// Every repo's default branch this process has looked up, by host and
/// repo home, so the linked worktrees of one repo share one answer. `None` is
/// both "asked, still waiting" and "asked, there is none".
// ponytail: never refreshed; a changed origin/HEAD shows after a restart.
#[derive(Default)]
struct DefaultBranches(HashMap<(HostId, PathBuf), Option<String>>);

impl Global for DefaultBranches {}

/// A group header's git: the rows' shared counts under the repo's default
/// branch (found from `repo`, the first row's checkout), or that branch
/// alone when the rows share no counts.
pub(crate) fn header_git(
    shared: Option<&SharedGit>,
    repo: Option<(HostId, PathBuf)>,
    cx: &mut Context<Tty7App>,
) -> Option<SharedGit> {
    let branch = repo.and_then(|(host, cwd)| default_branch(host, &cwd, cx));
    match (shared, branch) {
        (Some(s), branch) => Some(SharedGit {
            status: GitStatus {
                branch: branch.unwrap_or_default(),
                ..s.status.clone()
            },
            click: s.click.clone(),
            rows: s.rows.clone(),
        }),
        (None, Some(branch)) => Some(SharedGit {
            status: GitStatus {
                branch,
                added: 0,
                removed: 0,
            },
            click: None,
            rows: Vec::new(),
        }),
        (None, None) => None,
    }
}

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
    /// A click on a pinned group's pin. A folder group is unpinned: its tabs
    /// fall back to the auto groups their cwds resolve to. A label group has
    /// nothing to fall back to — the click deletes it, its name and the tabs
    /// picked for it by hand — so that asks first.
    pub(crate) fn pin_clicked(
        &mut self,
        id: GroupId,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if folder {
            return self.delete_group(id, cx);
        }
        let name = self
            .sidebar_groups
            .pinned
            .iter()
            .find(|g| g.id == id)
            .and_then(|g| g.name.clone())
            .unwrap_or_default();
        let answer = window.prompt(
            PromptLevel::Warning,
            &t_fmt(L10nKey::SidebarDeleteGroupTitle, &[("name", &name)]),
            Some(&t(L10nKey::SidebarDeleteGroupBody)),
            &crate::ui::confirm_answers(&t(L10nKey::SidebarDeleteGroup), &t(L10nKey::Cancel)),
            cx,
        );
        cx.spawn(async move |this, cx| {
            if matches!(answer.await, Ok(0)) {
                let _ = this.update(cx, |this, cx| this.delete_group(id, cx));
            }
        })
        .detach();
    }

    /// The group header options, as rows for Settings' Tabs group.
    pub(crate) fn group_header_settings(&self, cx: &mut Context<Self>) -> [AnyElement; 6] {
        let cfg = cx.global::<Config>();
        let (outline, fill) = (cfg.fork.group_outline, cfg.fork.group_background);
        let animations = cfg.fork.animations;
        let scope_ix = match cfg.fork.group_background_scope {
            GroupBackgroundScope::Header => 0,
            GroupBackgroundScope::Group => 1,
        };
        let scopes = [
            t(L10nKey::SettingsGroupScopeHeader),
            t(L10nKey::SettingsGroupScopeGroup),
        ];
        let ix = |s: GroupColorSource| match s {
            GroupColorSource::Theme => 0,
            GroupColorSource::Hashed => 1,
        };
        let (outline_ix, fill_ix) = (
            ix(cfg.fork.group_outline_color),
            ix(cfg.fork.group_background_color),
        );
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
                this.update_config(cx, |c| c.fork.group_outline = on)
            }),
            self.settings_choice(
                "wt-group-outline-color",
                &sources,
                outline_ix,
                cx,
                move |this, ix, _, cx| {
                    this.update_config(cx, |c| c.fork.group_outline_color = pick(ix))
                },
            ),
            self.settings_switch("wt-group-background", fill, cx, |this, on, _, cx| {
                this.update_config(cx, |c| c.fork.group_background = on)
            }),
            self.settings_choice(
                "wt-group-background-color",
                &sources,
                fill_ix,
                cx,
                move |this, ix, _, cx| {
                    this.update_config(cx, |c| c.fork.group_background_color = pick(ix))
                },
            ),
            self.settings_choice(
                "wt-group-background-scope",
                &scopes,
                scope_ix,
                cx,
                |this, ix, _, cx| {
                    this.update_config(cx, |c| {
                        c.fork.group_background_scope = match ix {
                            0 => GroupBackgroundScope::Header,
                            _ => GroupBackgroundScope::Group,
                        }
                    })
                },
            ),
            self.settings_switch("wt-group-animations", animations, cx, |this, on, _, cx| {
                this.update_config(cx, |c| c.fork.animations = on)
            }),
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
            (
                L10nKey::SettingsGroupBackgroundScope,
                L10nKey::SettingsGroupBackgroundScopeDesc,
            ),
            (
                L10nKey::SettingsGroupAnimations,
                L10nKey::SettingsGroupAnimationsDesc,
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

    #[test]
    fn theme_ignores_the_group_and_hashed_is_its_colour_with_the_override_winning() {
        let theme: Hsla = gpui::rgb(0x123456).into();
        let none = HashMap::new();
        let over = HashMap::from([("tty7".to_string(), "#ff8800".to_string())]);
        assert_eq!(
            source_color(
                GroupColorSource::Theme,
                "tty7",
                Some(1),
                &over,
                false,
                theme
            ),
            theme
        );
        assert_eq!(
            source_color(
                GroupColorSource::Hashed,
                "tty7",
                Some(1),
                &none,
                false,
                theme
            ),
            group_color("tty7", Some(1), &none, false)
        );
        assert_eq!(
            source_color(
                GroupColorSource::Hashed,
                "tty7",
                Some(1),
                &over,
                false,
                theme
            ),
            Hsla::from(gpui::rgb(0xff8800))
        );
    }

    #[test]
    fn exactly_one_of_header_and_block_wears_the_fill() {
        assert!(wears(GroupBackgroundScope::Header, false));
        assert!(!wears(GroupBackgroundScope::Header, true));
        assert!(!wears(GroupBackgroundScope::Group, false));
        assert!(wears(GroupBackgroundScope::Group, true));
    }

    #[test]
    fn a_fold_eases_from_one_end_to_the_other_and_rests_there() {
        let ms = Duration::from_millis;
        assert_eq!(openness_at(false, None), 1.);
        assert_eq!(openness_at(true, None), 0.);
        assert_eq!(openness_at(false, Some(ms(0))), 0.);
        assert_eq!(openness_at(true, Some(ms(0))), 1.);
        assert_eq!(openness_at(false, Some(FOLD / 2)), 0.5);
        assert_eq!(openness_at(false, Some(FOLD * 3)), 1.);
        assert_eq!(openness_at(true, Some(FOLD * 3)), 0.);
        // Ease in: the first tenth covers less than a tenth of the way.
        assert!(openness_at(false, Some(FOLD / 10)) < 0.1);
        let mut last = 0.;
        for step in 0..=20 {
            let now = openness_at(false, Some(FOLD * step / 20));
            assert!(now >= last, "step {step}");
            last = now;
        }
    }

    #[test]
    fn reversing_mid_fold_carries_on_from_where_it_was() {
        assert_eq!(restart(None), Duration::ZERO);
        assert_eq!(restart(Some(FOLD * 2)), Duration::ZERO);
        let at = FOLD / 4;
        let before = openness_at(false, Some(at));
        let after = openness_at(true, Some(restart(Some(at))));
        assert!((before - after).abs() < 1e-6, "{before} vs {after}");
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
