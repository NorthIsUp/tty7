//! The fork's global hotkey: a system-wide chord (`global_hotkey`, ⌥Space by
//! default) that shows and hides one dedicated tty7 window, iTerm2's hotkey
//! window. Only the hotkey shows it: ⌘Tab, the Dock and launch restore leave
//! it hidden, and the hotkey leaves every other tty7 window where it is.
//! Optionally over the whole screen the mouse is on
//! (`global_hotkey_fullscreen`), hidden on focus loss, faded in and out.
//!
//! macOS only: Carbon's `RegisterEventHotKey` needs no Accessibility grant.
//! Elsewhere `init` is a no-op and the Settings rows are not drawn.
//!
//! - [`carbon`]: the chord table and the Carbon calls.
//! - `appkit`: the window, the press, show / hide / fade / place / lift.
//! - `settings`: the Settings rows.

#[cfg(target_os = "macos")]
mod appkit;
mod carbon;
#[cfg(target_os = "macos")]
mod settings;

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
};
use gpui_component::menu::{PopupMenu, PopupMenuItem};

use crate::core::session::{WindowViews, WorkspaceId};
use crate::ui::app::Tty7App;
use crate::ui::i18n::{L10nKey, t};

/// What a hotkey press does, from where the hotkey window stands.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Toggle {
    /// Ordered out, or not open yet: bring it back, faded in.
    Show,
    /// On screen but not what has focus: raise it, no fade.
    Focus,
    /// It (or a window raised over it) has focus: order it out, and hand
    /// focus back to the app before.
    Hide,
}

/// The other tty7 windows over a floating hotkey window. Z-order follows
/// activation: a window that was up before the hotkey window stays under it,
/// one opened or clicked while it is up goes above it, and focusing the
/// hotkey window again puts it back on top.
struct Stack<W> {
    floating: bool,
    lifted: Vec<W>,
}

impl<W> Default for Stack<W> {
    fn default() -> Self {
        Self {
            floating: false,
            lifted: Vec::new(),
        }
    }
}

impl<W: PartialEq> Stack<W> {
    /// The hotkey window took focus: what was lifted over it goes back down.
    fn summoned(&mut self, floating: bool) -> Vec<W> {
        self.floating = floating;
        std::mem::take(&mut self.lifted)
    }

    /// Another window took focus: true when it has to go above.
    fn activated(&mut self, window: W) -> bool {
        if !self.floating || self.lifted.contains(&window) {
            return false;
        }
        self.lifted.push(window);
        true
    }

    /// Hidden, or dropped to the normal level: everything goes back down.
    fn dismissed(&mut self) -> Vec<W> {
        self.summoned(false)
    }

    fn lifted(&self) -> &[W] {
        &self.lifted
    }
}

/// The full screen hotkey window's level: NSFloatingWindowLevel, over other
/// apps' windows but under every system window. Force Quit and the out of
/// application memory dialog are loginwindow's, at kCGModalPanelWindowLevel
/// (8), so this and the windows lifted over it stay below that; the Dock gets
/// out of its way through presentation options, not a level.
const LEVEL: isize = 3;

/// kCGModalPanelWindowLevel, where loginwindow puts Force Quit.
#[cfg(test)]
const SYSTEM_MODAL_LEVEL: isize = 8;

/// The level a window activated over a floating hotkey window takes.
fn above(hotkey_level: isize) -> isize {
    hotkey_level + 1
}

/// How the Dock makes room for the full screen hotkey window, following the
/// user's own Dock setting: an auto-hiding Dock still slides in over it, an
/// always-shown one is hidden while it has focus. The menu bar stays.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Dock {
    AutoHide,
    Hide,
}

fn dock(autohides: bool) -> Dock {
    match autohides {
        true => Dock::AutoHide,
        false => Dock::Hide,
    }
}

/// How the app's presentation options change: `Some(on)` sets (`true`) or
/// puts back the Dock, `None` leaves it. They are on exactly
/// while the full screen hotkey window is the key window, and only what the
/// hotkey window set is ever put back, so options it never set (another
/// window in native full screen) are left alone.
fn presentation(presented: bool, hotkey_is_key: bool, fullscreen: bool) -> Option<bool> {
    let want = hotkey_is_key && fullscreen;
    (want != presented).then_some(want)
}

/// The full screen hotkey window's `(y, height)` on a screen, in AppKit's
/// bottom-up coordinates: from the screen's bottom, over the Dock it hides,
/// up to the top of `visibleFrame`, under the menu bar that screen shows.
fn cover(frame_y: f64, visible_y: f64, visible_height: f64) -> (f64, f64) {
    (frame_y, visible_y + visible_height - frame_y)
}

/// The workspace a launch or a Dock click reopens: never the hotkey window's.
pub(crate) fn to_restore(views: &WindowViews, hotkey: Option<WorkspaceId>) -> Option<WorkspaceId> {
    let Some(hotkey) = hotkey else {
        return views.workspace_to_restore();
    };
    let mut views = views.clone();
    views.views.retain(|w| w.id != hotkey);
    views.workspace_to_restore()
}

/// The hotkey window's workspace while the hotkey is on, which "most recent
/// window" and launch restore skip.
pub(crate) fn workspace(cx: &App) -> Option<WorkspaceId> {
    #[cfg(target_os = "macos")]
    return appkit::workspace(cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        None
    }
}

/// The switcher row menu's hotkey item for `row`: its label and the hotkey
/// workspace picking it leaves.
fn pick(hotkey: Option<WorkspaceId>, row: WorkspaceId) -> (L10nKey, Option<WorkspaceId>) {
    match hotkey == Some(row) {
        true => (L10nKey::SwitcherUnsetHotkeyWorkspace, None),
        false => (L10nKey::SwitcherSetHotkeyWorkspace, Some(row)),
    }
}

/// The chord a workspace list shows next to the hotkey workspace, if `row`
/// is it and the hotkey is on.
fn badge(spec: &Option<String>, hotkey: Option<WorkspaceId>, row: WorkspaceId) -> Option<&str> {
    configured(spec).filter(|_| hotkey == Some(row))
}

fn chord_of(cx: &App, row: WorkspaceId) -> Option<String> {
    let cfg = cx.try_global::<crate::core::config::Config>()?;
    badge(&cfg.fork.global_hotkey, workspace(cx), row).map(str::to_string)
}

/// The hotkey chord as keycaps, on the hotkey workspace's switcher row.
pub(crate) fn row_badge(cx: &App, row: WorkspaceId) -> Option<AnyElement> {
    let spec = chord_of(cx, row)?;
    Some(
        div()
            .id(("switcher-row-hotkey", row.element_key() as usize))
            .flex_shrink_0()
            .child(crate::ui::dialog::chord(
                crate::ui::keymap::key_tokens(&spec),
                cx,
            ))
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new(t(L10nKey::SwitcherHotkeyWorkspace))
                    .build(window, cx)
            })
            .into_any_element(),
    )
}

/// A Workspaces menu item's label, with the chord after the hotkey workspace.
pub(crate) fn menu_label(cx: &App, row: WorkspaceId, label: String) -> String {
    match chord_of(cx, row) {
        Some(spec) => format!("{label}  {}", crate::ui::keymap::key_tokens(&spec).concat()),
        None => label,
    }
}

/// "Set as Hotkey Workspace" / "Unset Hotkey Workspace" at the end of a
/// switcher row's workspace verbs, while the hotkey is on. A row this client
/// has not adopted (`managed` false) has no verbs to add it to.
pub(crate) fn menu_item(
    menu: PopupMenu,
    row: WorkspaceId,
    managed: bool,
    app: WeakEntity<Tty7App>,
    cx: &App,
) -> PopupMenu {
    let on = cx
        .try_global::<crate::core::config::Config>()
        .is_some_and(|c| configured(&c.fork.global_hotkey).is_some());
    if !cfg!(target_os = "macos") || !on || !managed {
        return menu;
    }
    let (label, next) = pick(workspace(cx), row);
    menu.separator()
        .item(PopupMenuItem::new(t(label)).on_click(move |_, window, cx| {
            let _ = app.update(cx, |this, cx| this.close_switcher(window, cx));
            set_workspace(cx, next);
        }))
}

/// Makes `id` the hotkey workspace (`None`: none, so the next press opens a
/// fresh one). The hotkey window it replaces goes back to a plain window,
/// hidden when another workspace takes over.
fn set_workspace(cx: &mut App, id: Option<WorkspaceId>) {
    #[cfg(target_os = "macos")]
    appkit::set_workspace(cx, id);
    #[cfg(not(target_os = "macos"))]
    let _ = (cx, id);
}

fn toggle(hidden: bool, frontmost: bool) -> Toggle {
    match (hidden, frontmost) {
        (true, _) => Toggle::Show,
        (false, true) => Toggle::Hide,
        (false, false) => Toggle::Focus,
    }
}

/// The configured chord, or `None` when it is off (`null` or `""`).
fn configured(spec: &Option<String>) -> Option<&str> {
    spec.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

pub fn init(cx: &mut App) {
    #[cfg(target_os = "macos")]
    appkit::init(cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        log::debug!("global hotkey: not supported on this platform");
    }
}

#[cfg(not(target_os = "macos"))]
impl crate::ui::app::Tty7App {
    pub(crate) fn hotkey_window_settings(
        &self,
        _cx: &mut gpui::Context<Self>,
    ) -> Option<gpui::AnyElement> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::CoreConfig as Config;

    #[test]
    fn the_hotkey_defaults_on_at_option_space_and_null_or_blank_turns_it_off() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(configured(&cfg.fork.global_hotkey), Some("alt-space"));
        assert!(!cfg.fork.global_hotkey_fullscreen);
        assert!(!cfg.fork.global_hotkey_hide_on_blur);
        assert_eq!(cfg.fork.global_hotkey_fade_ms, 150);
        assert_eq!(Config::default().fork.global_hotkey, cfg.fork.global_hotkey);

        let off: Config = serde_json::from_str(r#"{"global_hotkey": null}"#).unwrap();
        assert_eq!(configured(&off.fork.global_hotkey), None);
        let blank: Config = serde_json::from_str(r#"{"global_hotkey": " "}"#).unwrap();
        assert_eq!(configured(&blank.fork.global_hotkey), None);
        let set: Config = serde_json::from_str(
            r#"{"global_hotkey": "cmd-shift-t", "global_hotkey_fullscreen": true, "global_hotkey_fade_ms": 0}"#,
        )
        .unwrap();
        assert_eq!(configured(&set.fork.global_hotkey), Some("cmd-shift-t"));
        assert!(set.fork.global_hotkey_fullscreen);
        assert_eq!(set.fork.global_hotkey_fade_ms, 0);
    }

    #[test]
    fn a_press_shows_when_hidden_hides_when_frontmost_and_focuses_otherwise() {
        assert_eq!(toggle(true, false), Toggle::Show);
        assert_eq!(toggle(true, true), Toggle::Show);
        assert_eq!(toggle(false, true), Toggle::Hide);
        assert_eq!(toggle(false, false), Toggle::Focus);
    }

    #[test]
    fn a_window_goes_above_the_floating_hotkey_window_only_when_activated_after_it() {
        let mut stack = Stack::default();
        assert!(!stack.activated("settings"), "up before the hotkey window");
        assert!(stack.summoned(true).is_empty());
        assert!(stack.lifted().is_empty(), "the one open before stays below");
        assert!(stack.activated("settings"), "clicked while it is up");
        assert!(stack.activated("picker"), "opened while it is up");
        assert!(!stack.activated("picker"));
        assert_eq!(above(25), 26);
        assert_eq!(stack.summoned(true), ["settings", "picker"]);
        assert!(stack.activated("settings"));
        assert_eq!(stack.dismissed(), ["settings"]);
        assert!(!stack.activated("settings"), "hidden: nothing is lifted");
        let mut plain = Stack::default();
        plain.summoned(false);
        assert!(
            !plain.activated("settings"),
            "a normal-level window needs no lift"
        );
    }

    #[test]
    fn the_hotkey_window_and_what_is_lifted_over_it_stay_under_force_quit() {
        assert!(LEVEL > 0, "over other apps' normal windows");
        assert!(above(LEVEL) < SYSTEM_MODAL_LEVEL);
    }

    #[test]
    fn an_auto_hiding_dock_slides_over_the_hotkey_window_and_a_fixed_one_hides() {
        assert_eq!(dock(true), Dock::AutoHide);
        assert_eq!(dock(false), Dock::Hide);
    }

    #[test]
    fn the_full_screen_window_stays_under_the_menu_bar_and_covers_the_dock() {
        // 1000pt screen at y=0, 24pt menu bar, 70pt Dock at the bottom.
        assert_eq!(cover(0.0, 70.0, 906.0), (0.0, 976.0));
        // A second display above the first, no menu bar of its own.
        assert_eq!(cover(1000.0, 1000.0, 800.0), (1000.0, 800.0));
    }

    #[test]
    fn the_dock_follows_the_key_window() {
        assert_eq!(presentation(false, true, true), Some(true), "summoned");
        assert_eq!(presentation(true, true, true), None, "already set");
        assert_eq!(
            presentation(true, false, true),
            Some(false),
            "another tty7 window is key, or the hotkey window closed"
        );
        assert_eq!(presentation(false, true, false), None, "window style");
        assert_eq!(
            presentation(false, false, true),
            None,
            "never set, never touched"
        );
    }

    #[test]
    fn a_focus_change_inside_tty7_is_not_a_deactivation() {
        // A tab switch or another tty7 window taking key only reaches the
        // stack; only the app resigning active dismisses it.
        let mut stack = Stack::default();
        stack.summoned(true);
        assert!(stack.activated("other window"));
        assert_eq!(stack.summoned(true), ["other window"]);
        assert!(stack.activated("other window"), "still floating");
    }

    #[test]
    fn a_launch_or_dock_click_never_restores_the_hotkey_window() {
        use crate::core::session::WindowView;
        let (hot, other) = (WindowView::default(), WindowView::default());
        let (hot_id, other_id) = (hot.id, other.id);
        let views = WindowViews {
            views: vec![
                WindowView {
                    open: true,
                    last_active: 1,
                    ..other
                },
                WindowView {
                    open: true,
                    last_active: 2,
                    ..hot
                },
            ],
            active: Some(hot_id),
        };
        assert_eq!(to_restore(&views, None), Some(hot_id));
        assert_eq!(to_restore(&views, Some(hot_id)), Some(other_id));
        let alone = WindowViews {
            views: vec![views.views[1].clone()],
            active: Some(hot_id),
        };
        assert_eq!(to_restore(&alone, Some(hot_id)), None);
    }

    #[test]
    fn a_row_sets_itself_as_the_hotkey_workspace_and_the_current_one_unsets() {
        use crate::core::session::WindowView;
        let (a, b) = (WindowView::default().id, WindowView::default().id);
        assert_eq!(
            pick(None, a),
            (L10nKey::SwitcherSetHotkeyWorkspace, Some(a))
        );
        assert_eq!(
            pick(Some(b), a),
            (L10nKey::SwitcherSetHotkeyWorkspace, Some(a))
        );
        assert_eq!(
            pick(Some(a), a),
            (L10nKey::SwitcherUnsetHotkeyWorkspace, None)
        );
    }

    #[test]
    fn only_the_hotkey_workspace_row_gets_the_chord_and_only_while_the_hotkey_is_on() {
        use crate::core::session::WindowView;
        let (a, b) = (WindowView::default().id, WindowView::default().id);
        let on = Some("alt-space".to_string());
        assert_eq!(badge(&on, Some(a), a), Some("alt-space"));
        assert_eq!(badge(&on, Some(a), b), None);
        assert_eq!(badge(&on, None, a), None);
        assert_eq!(badge(&None, Some(a), a), None);
        assert_eq!(badge(&Some(" ".into()), Some(a), a), None);
    }
}
