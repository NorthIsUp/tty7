//! The hotkey window's Settings rows (macOS): on, the chord (recorded by
//! pressing it), full screen, hide on focus loss, and the fade.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::h_flex;

use super::appkit::{HotkeyWindow, sync};
use super::carbon::carbon_chord;
use super::configured;
use crate::core::config::Config;
use crate::ui::app::Tty7App;
use crate::ui::i18n::{L10nKey, t, t_fmt};
use crate::ui::settings::kit::{self, Tk, fs};

const FADE_BUCKETS: [u64; 5] = [0, 100, 150, 250, 400];
const FADE_LABELS: [&str; 5] = ["0 ms", "100 ms", "150 ms", "250 ms", "400 ms"];

impl Tty7App {
    pub(crate) fn hotkey_window_settings(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tk = Tk::of(cx);
        let cfg = cx.global::<Config>();
        let spec = configured(&cfg.fork.global_hotkey).map(str::to_string);
        let (fullscreen, hide_on_blur, fade) = (
            cfg.fork.global_hotkey_fullscreen,
            cfg.fork.global_hotkey_hide_on_blur,
            cfg.fork.global_hotkey_fade_ms,
        );
        let recording = cx
            .try_global::<HotkeyWindow>()
            .is_some_and(|hk| hk.recording.is_some());
        let on = self.settings_switch("hotkey-on", spec.is_some(), cx, |this, on, _, cx| {
            this.update_config(cx, |c| {
                c.fork.global_hotkey = on.then(|| "alt-space".to_string())
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
                this.update_config(cx, |c| c.fork.global_hotkey_fullscreen = on)
            });
        let blur = self.settings_switch(
            "hotkey-hide-on-blur",
            hide_on_blur,
            cx,
            |this, on, _, cx| this.update_config(cx, |c| c.fork.global_hotkey_hide_on_blur = on),
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
                    this.update_config(cx, |c| c.fork.global_hotkey_fade_ms = ms)
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
        let keys = cx.intercept_keystrokes(move |ev, _, cx| {
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
                Some(spec) => this.update_config(cx, |c| c.fork.global_hotkey = spec),
                None => cx.notify(),
            });
            sync(cx);
        });
        // Closing Settings mid-recording keeps the old chord, and gives the
        // keyboard back.
        let this = cx.weak_entity();
        let closed = cx.on_window_closed(move |cx, _| {
            if this
                .read_with(cx, |this, _| this.has_settings())
                .unwrap_or(false)
            {
                return;
            }
            cx.global_mut::<HotkeyWindow>().recording = None;
            sync(cx);
        });
        cx.global_mut::<HotkeyWindow>().recording = Some([keys, closed]);
        sync(cx);
        cx.notify();
    }
}
