//! The fork's global hotkey: a system-wide chord (`global_hotkey`, ⌥Space by
//! default) that shows and hides one dedicated tty7 window, iTerm2's hotkey
//! window. Only the hotkey shows it: ⌘Tab, the Dock and launch restore leave
//! it hidden, and the hotkey leaves every other tty7 window where it is.
//! Optionally over the whole screen the mouse is on
//! (`global_hotkey_fullscreen`), hidden on focus loss, faded in and out.
//!
//! macOS only: Carbon's `RegisterEventHotKey` needs no Accessibility grant.
//! Elsewhere `init` is a no-op and the Settings rows are not drawn.

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement as _, Keystroke, ParentElement as _,
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
    return mac::workspace(cx);
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
    badge(&cfg.global_hotkey, workspace(cx), row).map(str::to_string)
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

/// "Set as Hotkey Workspace" / "Unset Hotkey Workspace" on a switcher row,
/// while the hotkey is on.
pub(crate) fn menu_item(
    menu: PopupMenu,
    row: WorkspaceId,
    app: WeakEntity<Tty7App>,
    cx: &App,
) -> PopupMenu {
    let on = cx
        .try_global::<crate::core::config::Config>()
        .is_some_and(|c| configured(&c.global_hotkey).is_some());
    if !cfg!(target_os = "macos") || !on {
        return menu;
    }
    let (label, next) = pick(workspace(cx), row);
    menu.item(PopupMenuItem::new(t(label)).on_click(move |_, window, cx| {
        let _ = app.update(cx, |this, cx| this.close_switcher(window, cx));
        set_workspace(cx, next);
    }))
}

/// Makes `id` the hotkey workspace (`None`: none, so the next press opens a
/// fresh one). The hotkey window it replaces goes back to a plain window,
/// hidden when another workspace takes over.
pub(crate) fn set_workspace(cx: &mut App, id: Option<WorkspaceId>) {
    #[cfg(target_os = "macos")]
    mac::set_workspace(cx, id);
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

/// A keymap chord (`alt-space`) as Carbon's virtual key code and modifier
/// mask. `None` for a chord Carbon cannot register: a sequence, or a key off
/// the table.
pub(crate) fn carbon_chord(spec: &str) -> Option<(u32, u32)> {
    if spec.split_whitespace().count() != 1 {
        return None;
    }
    let ks = Keystroke::parse(spec).ok()?;
    let code = carbon_key_code(&ks.key)?;
    let m = ks.modifiers;
    let mods = [
        (m.platform, 0x0100), // cmdKey
        (m.shift, 0x0200),    // shiftKey
        (m.alt, 0x0800),      // optionKey
        (m.control, 0x1000),  // controlKey
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |acc, (_, bit)| acc | bit);
    Some((code, mods))
}

/// `kVK_*` from HIToolbox's Events.h, ANSI positions.
fn carbon_key_code(key: &str) -> Option<u32> {
    Some(match key {
        "a" => 0x00,
        "s" => 0x01,
        "d" => 0x02,
        "f" => 0x03,
        "h" => 0x04,
        "g" => 0x05,
        "z" => 0x06,
        "x" => 0x07,
        "c" => 0x08,
        "v" => 0x09,
        "b" => 0x0B,
        "q" => 0x0C,
        "w" => 0x0D,
        "e" => 0x0E,
        "r" => 0x0F,
        "y" => 0x10,
        "t" => 0x11,
        "1" => 0x12,
        "2" => 0x13,
        "3" => 0x14,
        "4" => 0x15,
        "6" => 0x16,
        "5" => 0x17,
        "=" => 0x18,
        "9" => 0x19,
        "7" => 0x1A,
        "-" => 0x1B,
        "8" => 0x1C,
        "0" => 0x1D,
        "]" => 0x1E,
        "o" => 0x1F,
        "u" => 0x20,
        "[" => 0x21,
        "i" => 0x22,
        "p" => 0x23,
        "enter" => 0x24,
        "l" => 0x25,
        "j" => 0x26,
        "'" => 0x27,
        "k" => 0x28,
        ";" => 0x29,
        "\\" => 0x2A,
        "," => 0x2B,
        "/" => 0x2C,
        "n" => 0x2D,
        "m" => 0x2E,
        "." => 0x2F,
        "tab" => 0x30,
        "space" => 0x31,
        "`" => 0x32,
        "backspace" => 0x33,
        "escape" => 0x35,
        "f5" => 0x60,
        "f6" => 0x61,
        "f7" => 0x62,
        "f3" => 0x63,
        "f8" => 0x64,
        "f9" => 0x65,
        "f11" => 0x67,
        "f10" => 0x6D,
        "f12" => 0x6F,
        "home" => 0x73,
        "pageup" => 0x74,
        "delete" => 0x75,
        "f4" => 0x76,
        "end" => 0x77,
        "f2" => 0x78,
        "pagedown" => 0x79,
        "f1" => 0x7A,
        "left" => 0x7B,
        "right" => 0x7C,
        "down" => 0x7D,
        "up" => 0x7E,
        _ => return None,
    })
}

pub fn init(cx: &mut App) {
    #[cfg(target_os = "macos")]
    mac::init(cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        log::debug!("global hotkey: not supported on this platform");
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use gpui::{
        AnyElement, AnyWindowHandle, App, AsyncApp, Context, Global, InteractiveElement as _,
        IntoElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
        Subscription, Window, div, px,
    };
    use gpui_component::h_flex;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationOptions, NSApplicationDidResignActiveNotification,
        NSEvent, NSNormalWindowLevel, NSRunningApplication, NSScreen, NSStatusWindowLevel, NSView,
        NSWindow, NSWindowCollectionBehavior, NSWindowDidBecomeKeyNotification, NSWindowLevel,
        NSWorkspace,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSRect};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::{Stack, Toggle, above, carbon_chord, configured, toggle};
    use crate::core::config::{Config, config_path, write_atomic};
    use crate::core::session::WorkspaceId;
    use crate::ui::app::Tty7App;
    use crate::ui::i18n::{L10nKey, t, t_fmt};
    use crate::ui::settings::kit::{self, Tk, fs};
    use crate::ui::windows::WindowRegistry;

    type OSStatus = i32;
    type Ref = *mut c_void;

    #[repr(C)]
    struct EventTypeSpec {
        class: u32,
        kind: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EventHotKeyID {
        signature: u32,
        id: u32,
    }

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn GetApplicationEventTarget() -> Ref;
        fn InstallEventHandler(
            target: Ref,
            handler: extern "C" fn(Ref, Ref, Ref) -> OSStatus,
            count: u32,
            types: *const EventTypeSpec,
            user_data: Ref,
            out: *mut Ref,
        ) -> OSStatus;
        fn RegisterEventHotKey(
            code: u32,
            modifiers: u32,
            id: EventHotKeyID,
            target: Ref,
            options: u32,
            out: *mut Ref,
        ) -> OSStatus;
        fn UnregisterEventHotKey(hotkey: Ref) -> OSStatus;
    }

    const KEYBOARD_CLASS: u32 = u32::from_be_bytes(*b"keyb");
    const HOTKEY_PRESSED: u32 = 5;
    /// The hotkey window's workspace id, so it comes back as the hotkey
    /// window after a restart rather than as the window a launch restores.
    const WORKSPACE_FILE: &str = "hotkey-window";

    #[derive(Clone, Copy)]
    enum Event {
        Pressed,
        KeyWindow,
        Resigned,
    }

    static EVENTS: OnceLock<smol::channel::Sender<Event>> = OnceLock::new();

    fn send(event: Event) {
        if let Some(tx) = EVENTS.get() {
            let _ = tx.try_send(event);
        }
    }

    extern "C" fn on_hotkey(_: Ref, _: Ref, _: Ref) -> OSStatus {
        send(Event::Pressed);
        0
    }

    /// What a modal summon changed on the window, put back when the style
    /// goes back to a plain window.
    struct Saved {
        window: AnyWindowHandle,
        frame: NSRect,
        level: NSWindowLevel,
        behavior: NSWindowCollectionBehavior,
    }

    /// A window activated over the floating hotkey window, and the level it
    /// goes back to.
    struct Lifted {
        ns: Retained<NSWindow>,
        level: NSWindowLevel,
        behavior: NSWindowCollectionBehavior,
    }

    impl PartialEq for Lifted {
        fn eq(&self, other: &Self) -> bool {
            Retained::as_ptr(&self.ns) == Retained::as_ptr(&other.ns)
        }
    }

    #[derive(Default)]
    struct HotkeyWindow {
        registered: Option<(String, usize)>,
        recording: Option<Subscription>,
        saved: Vec<Saved>,
        window: Option<AnyWindowHandle>,
        workspace: Option<WorkspaceId>,
        stack: Stack<Lifted>,
        /// The app that was frontmost when the hotkey showed the window, to
        /// get focus back when the hotkey hides it.
        previous: Option<Retained<NSRunningApplication>>,
        /// Bumped by every show and hide, so a fade still running from the
        /// last one stops rather than fighting this one.
        generation: u64,
    }

    impl Global for HotkeyWindow {}

    pub(super) fn workspace(cx: &App) -> Option<WorkspaceId> {
        let hk = cx.try_global::<HotkeyWindow>()?;
        configured(&cx.try_global::<Config>()?.global_hotkey)?;
        hk.workspace
    }

    pub(super) fn init(cx: &mut App) {
        let (tx, rx) = smol::channel::unbounded();
        if EVENTS.set(tx).is_err() {
            return;
        }
        let spec = EventTypeSpec {
            class: KEYBOARD_CLASS,
            kind: HOTKEY_PRESSED,
        };
        let mut handler = std::ptr::null_mut();
        // SAFETY: the handler is a plain fn that only touches a static.
        let status = unsafe {
            InstallEventHandler(
                GetApplicationEventTarget(),
                on_hotkey,
                1,
                &spec,
                std::ptr::null_mut(),
                &mut handler,
            )
        };
        if status != 0 {
            log::warn!("global hotkey: InstallEventHandler failed ({status})");
            return;
        }
        observe_appkit();
        cx.set_global(HotkeyWindow {
            workspace: load(),
            ..HotkeyWindow::default()
        });
        sync(cx);
        cx.observe_global::<Config>(sync).detach();
        cx.spawn(async move |cx: &mut AsyncApp| {
            while let Ok(event) = rx.recv().await {
                let cx2 = cx.clone();
                cx.update(|cx| match event {
                    Event::Pressed => pressed(cx, cx2),
                    Event::KeyWindow => key_window_changed(cx),
                    Event::Resigned => resigned(cx, cx2),
                });
            }
        })
        .detach();
    }

    /// The saved hotkey workspace; an empty file is none.
    fn load() -> Option<WorkspaceId> {
        config_path(WORKSPACE_FILE)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| s.trim().parse().ok())
    }

    fn save(id: Option<WorkspaceId>) {
        let text = id.map(|w| w.to_string()).unwrap_or_default();
        if let Some(path) = config_path(WORKSPACE_FILE)
            && let Err(e) = write_atomic(&path, text.as_bytes())
        {
            log::warn!("global hotkey: could not save its workspace: {e}");
        }
    }

    pub(super) fn set_workspace(cx: &mut App, id: Option<WorkspaceId>) {
        if cx
            .try_global::<HotkeyWindow>()
            .is_none_or(|hk| hk.workspace == id)
        {
            return;
        }
        if let Some((handle, ns)) = hotkey_window(cx) {
            release(cx, handle, &ns);
            if id.is_some() && ns.isVisible() {
                hide(cx, cx.to_async(), ns, false);
            }
        }
        save(id);
        let hk = cx.global_mut::<HotkeyWindow>();
        hk.window = None;
        hk.workspace = id;
        crate::ui::theme::set_menus(cx);
        cx.refresh_windows();
    }

    /// Undoes the hotkey window treatment: the full screen frame and level,
    /// and the windows lifted over it.
    fn release(cx: &mut App, handle: AnyWindowHandle, ns: &NSWindow) {
        let hk = cx.global_mut::<HotkeyWindow>();
        lower(hk.stack.dismissed());
        if let Some(ix) = hk.saved.iter().position(|s| s.window == handle) {
            let s = hk.saved.remove(ix);
            ns.setLevel(s.level);
            ns.setCollectionBehavior(s.behavior);
            ns.setFrame_display(s.frame, true);
        }
    }

    /// Any tty7 window taking focus (Settings, a file picker, another
    /// workspace), and tty7 as a whole losing it.
    fn observe_appkit() {
        let center = NSNotificationCenter::defaultCenter();
        // SAFETY: AppKit's notification name statics.
        let names = unsafe {
            [
                (NSWindowDidBecomeKeyNotification, Event::KeyWindow),
                (NSApplicationDidResignActiveNotification, Event::Resigned),
            ]
        };
        for (name, event) in names {
            let block = RcBlock::new(move |_: NonNull<NSNotification>| send(event));
            // SAFETY: no object filter; the block only touches a static.
            let token = unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
            };
            // Observed for the life of the app.
            std::mem::forget(token);
        }
    }

    /// Registers the configured chord if it is not the one already held. Off
    /// while Settings records a new one, or pressing it would fire it.
    fn sync(cx: &mut App) {
        let want = {
            let hk = cx.global::<HotkeyWindow>();
            if hk.recording.is_some() {
                None
            } else {
                configured(&cx.global::<Config>().global_hotkey).map(str::to_string)
            }
        };
        let hk = cx.global_mut::<HotkeyWindow>();
        if hk.registered.as_ref().map(|(s, _)| s) == want.as_ref() {
            return;
        }
        if let Some((spec, hotkey)) = hk.registered.take() {
            // SAFETY: `hotkey` came from RegisterEventHotKey and is released once.
            unsafe { UnregisterEventHotKey(hotkey as Ref) };
            log::info!("global hotkey: unregistered {spec}");
        }
        let Some(spec) = want else { return };
        let Some((code, modifiers)) = carbon_chord(&spec) else {
            log::warn!("global hotkey: {spec:?} is not a single chord macOS can register");
            return;
        };
        let id = EventHotKeyID {
            signature: u32::from_be_bytes(*b"tty7"),
            id: 1,
        };
        let mut hotkey = std::ptr::null_mut();
        // SAFETY: plain values in, an opaque ref out.
        let status = unsafe {
            RegisterEventHotKey(
                code,
                modifiers,
                id,
                GetApplicationEventTarget(),
                0,
                &mut hotkey,
            )
        };
        if status != 0 {
            log::warn!("global hotkey: {spec} is taken or refused ({status})");
            return;
        }
        log::info!("global hotkey: registered {spec}");
        hk.registered = Some((spec, hotkey as usize));
    }

    fn pressed(cx: &mut App, async_cx: AsyncApp) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let (handle, ns, fresh) = match hotkey_window(cx) {
            Some((handle, ns)) => (handle, ns, false),
            None => {
                let Some((handle, ns)) = open_hotkey_window(cx) else {
                    return;
                };
                (handle, ns, true)
            }
        };
        let on_top = NSApplication::sharedApplication(mtm).isActive()
            && (ns.isKeyWindow()
                || cx
                    .global::<HotkeyWindow>()
                    .stack
                    .lifted()
                    .iter()
                    .any(|l| l.ns.isKeyWindow()));
        match (fresh, toggle(!ns.isVisible(), on_top)) {
            (true, _) | (false, Toggle::Show) => show(cx, async_cx, handle, ns, true),
            (false, Toggle::Focus) => show(cx, async_cx, handle, ns, false),
            (false, Toggle::Hide) => hide(cx, async_cx, ns, true),
        }
    }

    /// This session's hotkey window, if it is still open.
    fn hotkey_window(cx: &mut App) -> Option<(AnyWindowHandle, Retained<NSWindow>)> {
        let handle = cx.global::<HotkeyWindow>().window?;
        let ns = handle.update(cx, |_, w, _| ns_window(w)).ok().flatten()?;
        Some((handle, ns))
    }

    /// The saved hotkey workspace's window, reopened with its tabs, or a new
    /// workspace for a first press.
    fn open_hotkey_window(cx: &mut App) -> Option<(AnyWindowHandle, Retained<NSWindow>)> {
        let saved = cx.global::<HotkeyWindow>().workspace;
        let handle = match saved.and_then(|id| WindowRegistry::window_for(cx, id)) {
            Some(handle) => handle,
            None => {
                let before = cx.windows();
                crate::ui::windows::open(cx, saved);
                cx.windows().into_iter().find(|w| !before.contains(w))?
            }
        };
        let (ns, workspace) = handle
            .update(cx, |_, w, cx| {
                let workspace = WindowRegistry::app_in(cx, w).map(|a| a.read(cx).workspace);
                ns_window(w).zip(workspace)
            })
            .ok()
            .flatten()?;
        if saved != Some(workspace) {
            save(Some(workspace));
        }
        let hk = cx.global_mut::<HotkeyWindow>();
        hk.window = Some(handle);
        hk.workspace = Some(workspace);
        Some((handle, ns))
    }

    fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
        let RawWindowHandle::AppKit(h) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        // SAFETY: gpui's AppKit handle is its live NSView.
        let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
        view.window()
    }

    fn fade_ms(cx: &App) -> u64 {
        cx.global::<Config>().global_hotkey_fade_ms.min(2000)
    }

    fn next_generation(cx: &mut App) -> u64 {
        let hk = cx.global_mut::<HotkeyWindow>();
        hk.generation = hk.generation.wrapping_add(1);
        hk.generation
    }

    fn lower(lifted: Vec<Lifted>) {
        for l in lifted {
            l.ns.setLevel(l.level);
            l.ns.setCollectionBehavior(l.behavior);
        }
    }

    fn show(
        cx: &mut App,
        async_cx: AsyncApp,
        handle: AnyWindowHandle,
        ns: Retained<NSWindow>,
        fade_in: bool,
    ) {
        let generation = next_generation(cx);
        let me = NSRunningApplication::currentApplication();
        let previous = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .filter(|app| app.processIdentifier() != me.processIdentifier());
        place(cx, handle, &ns);
        let hk = cx.global_mut::<HotkeyWindow>();
        hk.previous = previous;
        lower(hk.stack.summoned(ns.level() > NSNormalWindowLevel));
        let ms = if fade_in { fade_ms(cx) } else { 0 };
        if ms > 0 {
            ns.setAlphaValue(0.0);
        }
        ns.makeKeyAndOrderFront(None);
        // Only the key and main windows come forward, where NSApp's
        // activate would bring every tty7 window along.
        #[allow(deprecated)]
        me.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        cx.spawn(async move |cx| {
            fade(&async_cx, &ns, 0.0, 1.0, ms, generation).await;
            // ponytail: macOS 14 may refuse a self-activation it does not
            // trace to the user; then fall back to NSApp's, which does raise
            // every window.
            let _ = cx.update(|cx| {
                if let Some(mtm) = MainThreadMarker::new()
                    && !NSApplication::sharedApplication(mtm).isActive()
                    && cx.global::<HotkeyWindow>().generation == generation
                {
                    cx.activate(true);
                }
            });
        })
        .detach();
    }

    /// Orders out only the hotkey window. `give_back`: the hotkey hid it, so
    /// focus returns to the app it was summoned from.
    fn hide(cx: &mut App, async_cx: AsyncApp, ns: Retained<NSWindow>, give_back: bool) {
        let generation = next_generation(cx);
        let ms = fade_ms(cx);
        let hk = cx.global_mut::<HotkeyWindow>();
        lower(hk.stack.dismissed());
        let previous = hk.previous.take();
        cx.spawn(async move |_| {
            if fade(&async_cx, &ns, ns.alphaValue(), 0.0, ms, generation).await {
                ns.orderOut(None);
                if give_back {
                    give_focus_back(previous);
                }
            }
            ns.setAlphaValue(1.0);
        })
        .detach();
    }

    fn give_focus_back(previous: Option<Retained<NSRunningApplication>>) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let ns_app = NSApplication::sharedApplication(mtm);
        if !ns_app.isActive() {
            return;
        }
        match previous {
            Some(app) => {
                app.activateWithOptions(NSApplicationActivationOptions::empty());
            }
            None if !ns_app
                .windows()
                .iter()
                .any(|w| w.isVisible() && w.canBecomeMainWindow()) =>
            {
                ns_app.hide(None);
            }
            None => {}
        }
    }

    /// Steps the window's alpha from `from` to `to` over `ms`. False when a
    /// newer show or hide took over.
    async fn fade(
        cx: &AsyncApp,
        ns: &NSWindow,
        from: f64,
        to: f64,
        ms: u64,
        generation: u64,
    ) -> bool {
        let start = Instant::now();
        loop {
            if cx.update(|cx| cx.global::<HotkeyWindow>().generation) != generation {
                return false;
            }
            let t = if ms == 0 {
                1.0
            } else {
                (start.elapsed().as_secs_f64() * 1000.0 / ms as f64).min(1.0)
            };
            ns.setAlphaValue(from + (to - from) * t);
            if t >= 1.0 {
                return true;
            }
            cx.background_executor()
                .timer(Duration::from_millis(8))
                .await;
        }
    }

    /// Fullscreen style: cover the screen the mouse is on, above other apps,
    /// on whichever Space is current. Window style: undo that if it was done.
    fn place(cx: &mut App, handle: AnyWindowHandle, ns: &NSWindow) {
        let fullscreen = cx.global::<Config>().global_hotkey_fullscreen;
        let hk = cx.global_mut::<HotkeyWindow>();
        let at = hk.saved.iter().position(|s| s.window == handle);
        if !fullscreen {
            if let Some(ix) = at {
                let s = hk.saved.remove(ix);
                ns.setLevel(s.level);
                ns.setCollectionBehavior(s.behavior);
                ns.setFrame_display(s.frame, true);
            }
            return;
        }
        if at.is_none() {
            hk.saved.push(Saved {
                window: handle,
                frame: ns.frame(),
                level: ns.level(),
                behavior: ns.collectionBehavior(),
            });
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let mouse = NSEvent::mouseLocation();
        let screens = NSScreen::screens(mtm);
        let screen = screens
            .iter()
            .find(|s| {
                let f = s.frame();
                mouse.x >= f.origin.x
                    && mouse.x < f.origin.x + f.size.width
                    && mouse.y >= f.origin.y
                    && mouse.y < f.origin.y + f.size.height
            })
            .or_else(|| NSScreen::mainScreen(mtm));
        let Some(screen) = screen else { return };
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        ns.setLevel(NSStatusWindowLevel);
        ns.setFrame_display(screen.frame(), true);
    }

    /// The hotkey window taking focus goes back on top; any other tty7
    /// window taking focus while it floats goes above it.
    fn key_window_changed(cx: &mut App) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(key) = NSApplication::sharedApplication(mtm).keyWindow() else {
            return;
        };
        let Some((_, hot)) = hotkey_window(cx) else {
            return;
        };
        log::debug!(
            "global hotkey: key window {:?} (level {}), hotkey window level {}",
            Retained::as_ptr(&key),
            key.level(),
            hot.level()
        );
        let stack = &mut cx.global_mut::<HotkeyWindow>().stack;
        if Retained::as_ptr(&key) == Retained::as_ptr(&hot) {
            lower(stack.summoned(hot.level() > NSNormalWindowLevel));
            return;
        }
        let lifted = Lifted {
            ns: key.clone(),
            level: key.level(),
            behavior: key.collectionBehavior(),
        };
        if stack.activated(lifted) {
            key.setCollectionBehavior(hot.collectionBehavior());
            key.setLevel(above(hot.level()));
        }
    }

    /// tty7 lost focus: hide the hotkey window (`global_hotkey_hide_on_blur`),
    /// or at least drop it back to the normal level so the app switched to
    /// is not stuck underneath it.
    fn resigned(cx: &mut App, async_cx: AsyncApp) {
        let Some((_, ns)) = hotkey_window(cx) else {
            return;
        };
        if !ns.isVisible() {
            return;
        }
        log::debug!("global hotkey: tty7 resigned active");
        if cx.global::<Config>().global_hotkey_hide_on_blur {
            hide(cx, async_cx, ns, false);
            return;
        }
        if ns.level() > NSNormalWindowLevel {
            ns.setLevel(NSNormalWindowLevel);
        }
        lower(cx.global_mut::<HotkeyWindow>().stack.dismissed());
    }

    const FADE_BUCKETS: [u64; 5] = [0, 100, 150, 250, 400];
    const FADE_LABELS: [&str; 5] = ["0 ms", "100 ms", "150 ms", "250 ms", "400 ms"];

    impl Tty7App {
        pub(crate) fn hotkey_window_settings(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
            let tk = Tk::of(cx);
            let cfg = cx.global::<Config>();
            let spec = configured(&cfg.global_hotkey).map(str::to_string);
            let (fullscreen, hide_on_blur, fade) = (
                cfg.global_hotkey_fullscreen,
                cfg.global_hotkey_hide_on_blur,
                cfg.global_hotkey_fade_ms,
            );
            let recording = cx
                .try_global::<HotkeyWindow>()
                .is_some_and(|hk| hk.recording.is_some());
            let on = self.settings_switch("hotkey-on", spec.is_some(), cx, |this, on, _, cx| {
                this.update_config(cx, |c| {
                    c.global_hotkey = on.then(|| "alt-space".to_string())
                })
            });
            let ring = if recording {
                vec![kit::ring(tk.fg, 1., true)]
            } else {
                vec![kit::ring(tk.k15, 0.5, true)]
            };
            let shown = match (&spec, recording) {
                (_, true) => div()
                    .text_size(fs(12.))
                    .text_color(tk.k45)
                    .child(t(L10nKey::SettingsPressKeysShort)),
                (Some(spec), false) => Self::keycap_row(spec, &tk),
                (None, false) => div().text_size(fs(12.)).text_color(tk.k3).child("—"),
            };
            let keys = h_flex()
                .id("hotkey-record")
                .h(px(26.))
                .min_w(px(72.))
                .px(px(8.))
                .gap(px(5.))
                .items_center()
                .justify_end()
                .rounded(px(6.))
                .cursor_pointer()
                .shadow(ring)
                .child(shown)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_hotkey_recording(cx)))
                .into_any_element();
            let fullscreen =
                self.settings_switch("hotkey-fullscreen", fullscreen, cx, |this, on, _, cx| {
                    this.update_config(cx, |c| c.global_hotkey_fullscreen = on)
                });
            let blur = self.settings_switch(
                "hotkey-hide-on-blur",
                hide_on_blur,
                cx,
                |this, on, _, cx| this.update_config(cx, |c| c.global_hotkey_hide_on_blur = on),
            );
            let fade_ix = FADE_BUCKETS.iter().position(|&b| b == fade);
            let custom = fade_ix.is_none().then(|| {
                t_fmt(
                    L10nKey::SettingsCustomValue,
                    &[("value", &format!("{fade} ms"))],
                )
            });
            let fade = self.settings_choice_valued(
                "hotkey-fade",
                &FADE_LABELS,
                fade_ix,
                custom,
                cx,
                |this, ix, _, cx| {
                    if let Some(&ms) = FADE_BUCKETS.get(ix) {
                        this.update_config(cx, |c| c.global_hotkey_fade_ms = ms)
                    }
                },
            );
            let rows = [
                (L10nKey::SettingsHotkeyOn, L10nKey::SettingsHotkeyOnDesc, on),
                (
                    L10nKey::SettingsHotkeyKey,
                    L10nKey::SettingsHotkeyKeyDesc,
                    keys,
                ),
                (
                    L10nKey::SettingsHotkeyFullscreen,
                    L10nKey::SettingsHotkeyFullscreenDesc,
                    fullscreen,
                ),
                (
                    L10nKey::SettingsHotkeyHideOnBlur,
                    L10nKey::SettingsHotkeyHideOnBlurDesc,
                    blur,
                ),
                (
                    L10nKey::SettingsHotkeyFade,
                    L10nKey::SettingsHotkeyFadeDesc,
                    fade,
                ),
            ]
            .map(|(label, desc, control)| {
                self.settings_row(t(label), t(desc), control, cx)
                    .into_any_element()
            });
            Some(self.settings_group(Some(t(L10nKey::SettingsHotkeyWindow)), None, rows, cx))
        }

        /// Click the keys, press a chord: Esc keeps the old one, Backspace
        /// turns the hotkey off.
        fn toggle_hotkey_recording(&mut self, cx: &mut Context<Self>) {
            if cx.global_mut::<HotkeyWindow>().recording.take().is_some() {
                sync(cx);
                cx.notify();
                return;
            }
            let this = cx.weak_entity();
            let sub = cx.intercept_keystrokes(move |ev, _, cx| {
                cx.stop_propagation();
                let ks = &ev.keystroke;
                let picked = match ks.key.as_str() {
                    "escape" => None,
                    "backspace" | "delete" if !ks.modifiers.modified() => Some(None),
                    _ => {
                        let spec = ks.unparse();
                        if carbon_chord(&spec).is_none() {
                            return;
                        }
                        Some(Some(spec))
                    }
                };
                cx.global_mut::<HotkeyWindow>().recording = None;
                let _ = this.update(cx, |this, cx| match picked {
                    Some(spec) => this.update_config(cx, |c| c.global_hotkey = spec),
                    None => cx.notify(),
                });
                sync(cx);
            });
            cx.global_mut::<HotkeyWindow>().recording = Some(sub);
            sync(cx);
            cx.notify();
        }
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
        assert_eq!(configured(&cfg.global_hotkey), Some("alt-space"));
        assert!(!cfg.global_hotkey_fullscreen);
        assert!(!cfg.global_hotkey_hide_on_blur);
        assert_eq!(cfg.global_hotkey_fade_ms, 150);
        assert_eq!(Config::default().global_hotkey, cfg.global_hotkey);

        let off: Config = serde_json::from_str(r#"{"global_hotkey": null}"#).unwrap();
        assert_eq!(configured(&off.global_hotkey), None);
        let blank: Config = serde_json::from_str(r#"{"global_hotkey": " "}"#).unwrap();
        assert_eq!(configured(&blank.global_hotkey), None);
        let set: Config = serde_json::from_str(
            r#"{"global_hotkey": "cmd-shift-t", "global_hotkey_fullscreen": true, "global_hotkey_fade_ms": 0}"#,
        )
        .unwrap();
        assert_eq!(configured(&set.global_hotkey), Some("cmd-shift-t"));
        assert!(set.global_hotkey_fullscreen);
        assert_eq!(set.global_hotkey_fade_ms, 0);
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
