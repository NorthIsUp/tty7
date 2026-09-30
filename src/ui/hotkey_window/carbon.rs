//! Carbon's system-wide hot key: `RegisterEventHotKey` needs no
//! Accessibility grant. The chord table is plain data, so it is tested on
//! every platform; the calls are macOS only.

use gpui::Keystroke;

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

#[cfg(target_os = "macos")]
pub(super) use ffi::{HotKey, install, register};

#[cfg(target_os = "macos")]
mod ffi {
    use std::ffi::c_void;
    use std::sync::OnceLock;

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

    static ON_PRESS: OnceLock<fn()> = OnceLock::new();

    extern "C" fn on_hotkey(_: Ref, _: Ref, _: Ref) -> OSStatus {
        if let Some(on_press) = ON_PRESS.get() {
            on_press();
        }
        0
    }

    /// Calls `on_press` for every press of a registered hot key, from the
    /// app's event loop. Once per process; the status Carbon refused with.
    pub(in super::super) fn install(on_press: fn()) -> Result<(), OSStatus> {
        if ON_PRESS.set(on_press).is_err() {
            return Ok(());
        }
        let spec = EventTypeSpec {
            class: KEYBOARD_CLASS,
            kind: HOTKEY_PRESSED,
        };
        let mut handler = std::ptr::null_mut();
        // SAFETY: the handler is a plain fn that only reads a static.
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
        match status {
            0 => Ok(()),
            refused => Err(refused),
        }
    }

    /// A registered hot key, unregistered when dropped.
    pub(in super::super) struct HotKey(Ref);

    impl Drop for HotKey {
        fn drop(&mut self) {
            // SAFETY: `self.0` came from RegisterEventHotKey and is released once.
            unsafe { UnregisterEventHotKey(self.0) };
        }
    }

    /// Registers `code` + `modifiers` ([`super::carbon_chord`]); the status
    /// when the chord is taken or refused.
    pub(in super::super) fn register(code: u32, modifiers: u32) -> Result<HotKey, OSStatus> {
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
        match status {
            0 => Ok(HotKey(hotkey)),
            refused => Err(refused),
        }
    }
}
