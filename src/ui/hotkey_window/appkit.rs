//! The hotkey window itself (macOS): its global state, the press, and the
//! AppKit work of showing, fading, placing and lifting windows.

use std::ptr::NonNull;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use block2::RcBlock;
use gpui::{AnyWindowHandle, App, AsyncApp, Global, Subscription, Window};
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

use super::carbon::{self, Plan, carbon_chord};
use super::{Stack, Toggle, above, configured, toggle};
use crate::core::config::{Config, config_path, write_atomic};
use crate::core::session::WorkspaceId;
use crate::ui::windows::WindowRegistry;

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
    registered: Option<(String, carbon::HotKey)>,
    /// A chord macOS refused, not asked again until the config names another.
    refused: Option<String>,
    /// Settings is recording a new chord (its keystrokes, and Settings
    /// closing): the hotkey is off meanwhile.
    recording: Option<[Subscription; 2]>,
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
    configured(&cx.try_global::<Config>()?.fork.global_hotkey)?;
    hk.workspace
}

pub(super) fn init(cx: &mut App) {
    let (tx, rx) = smol::channel::unbounded();
    if EVENTS.set(tx).is_err() {
        return;
    }
    if let Err(status) = carbon::install(|| send(Event::Pressed)) {
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
            cx.update(|cx| match event {
                Event::Pressed => pressed(cx),
                Event::KeyWindow => key_window_changed(cx),
                Event::Resigned => resigned(cx),
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
            // Out at once, not faded: a fade of a window no longer the
            // hotkey window's could be cut short by the next press and leave
            // it on screen half transparent.
            next_generation(cx);
            ns.orderOut(None);
            ns.setAlphaValue(1.0);
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
    restore_saved(hk, handle, ns);
}

/// Puts back what a full screen summon changed on `handle`'s window.
fn restore_saved(hk: &mut HotkeyWindow, handle: AnyWindowHandle, ns: &NSWindow) {
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

/// Whether Settings is recording a chord.
pub(super) fn recording(cx: &App) -> bool {
    cx.try_global::<HotkeyWindow>()
        .is_some_and(|hk| hk.recording.is_some())
}

/// Starts (`Some`: its keystroke and window-closed subscriptions) or ends
/// recording a chord, and registers what that leaves wanted.
pub(super) fn set_recording(cx: &mut App, recording: Option<[Subscription; 2]>) {
    cx.global_mut::<HotkeyWindow>().recording = recording;
    sync(cx);
}

/// Registers the configured chord if it is not the one already held. Off
/// while Settings records a new one, or pressing it would fire it. A chord
/// macOS refused is not asked for again until the hotkey is turned off or
/// another is configured.
fn sync(cx: &mut App) {
    let want = {
        let hk = cx.global::<HotkeyWindow>();
        if hk.recording.is_some() {
            None
        } else {
            configured(&cx.global::<Config>().fork.global_hotkey).map(str::to_string)
        }
    };
    let hk = cx.global_mut::<HotkeyWindow>();
    let registered = hk.registered.as_ref().map(|(s, _)| s.as_str());
    let plan = carbon::plan(registered, hk.refused.as_deref(), want.as_deref());
    if plan == Plan::Keep {
        return;
    }
    if let Some((spec, hotkey)) = hk.registered.take() {
        drop(hotkey);
        log::info!("global hotkey: unregistered {spec}");
    }
    let (Plan::Register, Some(spec)) = (plan, want) else {
        if plan == Plan::Off {
            hk.refused = None;
        }
        return;
    };
    hk.refused = None;
    let Some((code, modifiers)) = carbon_chord(&spec) else {
        log::warn!("global hotkey: {spec:?} is not a single chord macOS can register");
        hk.refused = Some(spec);
        return;
    };
    match carbon::register(code, modifiers) {
        Ok(hotkey) => {
            log::info!("global hotkey: registered {spec}");
            hk.registered = Some((spec, hotkey));
        }
        Err(status) => {
            log::warn!("global hotkey: {spec} is taken or refused ({status})");
            hk.refused = Some(spec);
        }
    }
}

fn pressed(cx: &mut App) {
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
        (true, _) | (false, Toggle::Show) => show(cx, handle, ns, true),
        (false, Toggle::Focus) => show(cx, handle, ns, false),
        (false, Toggle::Hide) => hide(cx, ns, true),
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
            // A fresh workspace's id is minted in there, so the new window
            // is the one the registry did not have before. None when the
            // open failed, rather than some other window.
            let before: Vec<WorkspaceId> = WindowRegistry::open_windows(cx)
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            crate::ui::windows::open(cx, saved);
            let (opened, _) = WindowRegistry::open_windows(cx)
                .into_iter()
                .find(|(id, _)| !before.contains(id))?;
            WindowRegistry::window_for(cx, opened)?
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
    let RawWindowHandle::AppKit(h) = HasWindowHandle::window_handle(window).ok()?.as_raw() else {
        return None;
    };
    // SAFETY: gpui's AppKit handle is its live NSView.
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

fn fade_ms(cx: &App) -> u64 {
    cx.global::<Config>().fork.global_hotkey_fade_ms.min(2000)
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

fn show(cx: &mut App, handle: AnyWindowHandle, ns: Retained<NSWindow>, fade_in: bool) {
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
        fade(cx, &ns, 0.0, 1.0, ms, generation).await;
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
fn hide(cx: &mut App, ns: Retained<NSWindow>, give_back: bool) {
    let generation = next_generation(cx);
    let ms = fade_ms(cx);
    let hk = cx.global_mut::<HotkeyWindow>();
    lower(hk.stack.dismissed());
    let previous = hk.previous.take();
    cx.spawn(async move |cx| {
        // A show that took over mid-fade owns the alpha now.
        if fade(cx, &ns, ns.alphaValue(), 0.0, ms, generation).await {
            ns.orderOut(None);
            ns.setAlphaValue(1.0);
            if give_back {
                give_focus_back(previous);
            }
        }
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
async fn fade(cx: &AsyncApp, ns: &NSWindow, from: f64, to: f64, ms: u64, generation: u64) -> bool {
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
    let fullscreen = cx.global::<Config>().fork.global_hotkey_fullscreen;
    let hk = cx.global_mut::<HotkeyWindow>();
    if !fullscreen {
        restore_saved(hk, handle, ns);
        return;
    }
    if !hk.saved.iter().any(|s| s.window == handle) {
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
fn resigned(cx: &mut App) {
    let Some((_, ns)) = hotkey_window(cx) else {
        return;
    };
    if !ns.isVisible() {
        return;
    }
    log::debug!("global hotkey: tty7 resigned active");
    if cx.global::<Config>().fork.global_hotkey_hide_on_blur {
        hide(cx, ns, false);
        return;
    }
    if ns.level() > NSNormalWindowLevel {
        ns.setLevel(NSNormalWindowLevel);
    }
    lower(cx.global_mut::<HotkeyWindow>().stack.dismissed());
}
