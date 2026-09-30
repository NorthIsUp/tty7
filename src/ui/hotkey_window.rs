//! The fork's global hotkey: a system-wide chord (`global_hotkey`, ⌥Space by
//! default) that brings tty7 to the front and hides it again, iTerm2's hotkey
//! window. Optionally over the whole screen the mouse is on
//! (`global_hotkey_fullscreen`), hidden on focus loss, faded in and out.
//!
//! macOS only: Carbon's `RegisterEventHotKey` needs no Accessibility grant.
//! Elsewhere `init` is a no-op and the Settings rows are not drawn.

use gpui::{App, Keystroke};

/// What a hotkey press does, from where tty7 stands when it lands.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Toggle {
    /// Hidden or with no window on screen: bring it back, faded in.
    Show,
    /// On screen behind another app: raise it, no fade.
    Focus,
    /// Frontmost: hide, and macOS hands focus back to the app before.
    Hide,
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
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use gpui::{
        AnyElement, AnyWindowHandle, App, AsyncApp, Context, Global, InteractiveElement as _,
        IntoElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
        Subscription, Window, div, px,
    };
    use gpui_component::h_flex;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSApplication, NSEvent, NSNormalWindowLevel, NSScreen, NSStatusWindowLevel, NSView,
        NSWindow, NSWindowCollectionBehavior, NSWindowLevel,
    };
    use objc2_foundation::NSRect;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::{Toggle, carbon_chord, configured, toggle};
    use crate::core::config::Config;
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

    static PRESSED: OnceLock<smol::channel::Sender<()>> = OnceLock::new();

    extern "C" fn on_hotkey(_: Ref, _: Ref, _: Ref) -> OSStatus {
        if let Some(tx) = PRESSED.get() {
            let _ = tx.try_send(());
        }
        0
    }

    /// What a modal summon changed on a window, put back when the style goes
    /// back to a plain window.
    struct Saved {
        window: AnyWindowHandle,
        frame: NSRect,
        level: NSWindowLevel,
        behavior: NSWindowCollectionBehavior,
    }

    #[derive(Default)]
    struct HotkeyWindow {
        registered: Option<(String, usize)>,
        recording: Option<Subscription>,
        saved: Vec<Saved>,
        watched: Vec<(AnyWindowHandle, Subscription)>,
        /// Bumped by every show and hide, so a fade still running from the
        /// last one stops rather than fighting this one.
        generation: u64,
    }

    impl Global for HotkeyWindow {}

    pub(super) fn init(cx: &mut App) {
        let (tx, rx) = smol::channel::unbounded();
        if PRESSED.set(tx).is_err() {
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
        cx.set_global(HotkeyWindow::default());
        sync(cx);
        cx.observe_global::<Config>(sync).detach();
        cx.spawn(async move |cx: &mut AsyncApp| {
            while rx.recv().await.is_ok() {
                let cx2 = cx.clone();
                cx.update(|cx| pressed(cx, cx2));
            }
        })
        .detach();
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
        let Some(workspace) = WindowRegistry::most_recent(cx) else {
            crate::ui::windows::reopen(cx);
            return;
        };
        let Some(handle) = WindowRegistry::window_for(cx, workspace) else {
            return;
        };
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let ns_app = NSApplication::sharedApplication(mtm);
        let Some(ns) = handle.update(cx, |_, w, _| ns_window(w)).ok().flatten() else {
            return;
        };
        let hidden = ns_app.isHidden() || !ns.isVisible();
        match toggle(hidden, ns_app.isActive()) {
            Toggle::Hide => hide(cx, async_cx, ns),
            Toggle::Show => show(cx, async_cx, workspace, handle, ns, true),
            Toggle::Focus => show(cx, async_cx, workspace, handle, ns, false),
        }
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

    fn show(
        cx: &mut App,
        async_cx: AsyncApp,
        workspace: WorkspaceId,
        handle: AnyWindowHandle,
        ns: Retained<NSWindow>,
        fade_in: bool,
    ) {
        let generation = next_generation(cx);
        place(cx, handle, &ns);
        watch_blur(cx, workspace, handle);
        let ms = if fade_in { fade_ms(cx) } else { 0 };
        if ms > 0 {
            ns.setAlphaValue(0.0);
        }
        cx.activate(true);
        let _ = handle.update(cx, |_, w, _| w.activate_window());
        cx.spawn(async move |_| {
            fade(&async_cx, &ns, 0.0, 1.0, ms, generation).await;
        })
        .detach();
    }

    fn hide(cx: &mut App, async_cx: AsyncApp, ns: Retained<NSWindow>) {
        let generation = next_generation(cx);
        let ms = fade_ms(cx);
        cx.spawn(async move |cx| {
            if fade(&async_cx, &ns, ns.alphaValue(), 0.0, ms, generation).await {
                cx.update(|cx| cx.hide());
            }
            // Hidden now, so the window comes back opaque from the Dock too.
            ns.setAlphaValue(1.0);
        })
        .detach();
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

    /// Once per window: when tty7 as a whole loses focus, hide it
    /// (`global_hotkey_hide_on_blur`), or at least drop a modal window back to
    /// the normal level so the app switched to is not stuck underneath it.
    fn watch_blur(cx: &mut App, workspace: WorkspaceId, handle: AnyWindowHandle) {
        if cx
            .global::<HotkeyWindow>()
            .watched
            .iter()
            .any(|(w, _)| *w == handle)
        {
            return;
        }
        let Some(app) = WindowRegistry::app_for(cx, workspace).and_then(|a| a.upgrade()) else {
            return;
        };
        let sub = handle.update(cx, |_, window, cx| {
            app.update(cx, |_, acx| {
                acx.observe_window_activation(window, move |_, window, cx| {
                    if window.is_window_active() {
                        return;
                    }
                    // The app's own active flag settles after the window's.
                    cx.spawn(async move |_, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                        let async_cx = cx.clone();
                        cx.update(|cx| blurred(cx, async_cx, handle));
                    })
                    .detach();
                })
            })
        });
        if let Ok(sub) = sub {
            cx.global_mut::<HotkeyWindow>().watched.push((handle, sub));
        }
    }

    fn blurred(cx: &mut App, async_cx: AsyncApp, handle: AnyWindowHandle) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let ns_app = NSApplication::sharedApplication(mtm);
        if ns_app.isActive() || ns_app.isHidden() {
            return;
        }
        let Some(ns) = handle.update(cx, |_, w, _| ns_window(w)).ok().flatten() else {
            return;
        };
        if cx.global::<Config>().global_hotkey_hide_on_blur {
            hide(cx, async_cx, ns);
        } else if cx
            .global::<HotkeyWindow>()
            .saved
            .iter()
            .any(|s| s.window == handle)
        {
            ns.setLevel(NSNormalWindowLevel);
        }
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
}
