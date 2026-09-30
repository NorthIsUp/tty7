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

#[cfg(test)]
use carbon::carbon_chord;

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
pub(crate) enum Toggle {
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
pub(crate) struct Stack<W> {
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
    pub(crate) fn summoned(&mut self, floating: bool) -> Vec<W> {
        self.floating = floating;
        std::mem::take(&mut self.lifted)
    }

    /// Another window took focus: true when it has to go above.
    pub(crate) fn activated(&mut self, window: W) -> bool {
        if !self.floating || self.lifted.contains(&window) {
            return false;
        }
        self.lifted.push(window);
        true
    }

    /// Hidden, or dropped to the normal level: everything goes back down.
    pub(crate) fn dismissed(&mut self) -> Vec<W> {
        self.summoned(false)
    }

    pub(crate) fn lifted(&self) -> &[W] {
        &self.lifted
    }
}

/// The level a window activated over a floating hotkey window takes.
pub(crate) fn above(hotkey_level: isize) -> isize {
    hotkey_level + 1
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
pub(crate) fn pick(
    hotkey: Option<WorkspaceId>,
    row: WorkspaceId,
) -> (L10nKey, Option<WorkspaceId>) {
    match hotkey == Some(row) {
        true => (L10nKey::SwitcherUnsetHotkeyWorkspace, None),
        false => (L10nKey::SwitcherSetHotkeyWorkspace, Some(row)),
    }
}

/// The chord a workspace list shows next to the hotkey workspace, if `row`
/// is it and the hotkey is on.
pub(crate) fn badge(
    spec: &Option<String>,
    hotkey: Option<WorkspaceId>,
    row: WorkspaceId,
) -> Option<&str> {
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
pub(crate) fn set_workspace(cx: &mut App, id: Option<WorkspaceId>) {
    #[cfg(target_os = "macos")]
    appkit::set_workspace(cx, id);
    #[cfg(not(target_os = "macos"))]
    let _ = (cx, id);
}

pub(crate) fn toggle(hidden: bool, frontmost: bool) -> Toggle {
    match (hidden, frontmost) {
        (true, _) => Toggle::Show,
        (false, true) => Toggle::Hide,
        (false, false) => Toggle::Focus,
    }
}

/// The configured chord, or `None` when it is off (`null` or `""`).
pub(crate) fn configured(spec: &Option<String>) -> Option<&str> {
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
    fn a_chord_maps_to_carbon_key_code_and_modifiers() {
        assert_eq!(carbon_chord("alt-space"), Some((0x31, 0x0800)));
        assert_eq!(carbon_chord("cmd-shift-t"), Some((0x11, 0x0100 | 0x0200)));
        assert_eq!(carbon_chord("ctrl-`"), Some((0x32, 0x1000)));
        assert_eq!(carbon_chord("f12"), Some((0x6F, 0)));
        assert_eq!(carbon_chord("ctrl-b x"), None);
        assert_eq!(carbon_chord("alt-nosuchkey"), None);
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
