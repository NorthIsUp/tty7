//! The fork's sidebar group colours, kept out of `tab_sidebar.rs` so rebasing
//! on upstream touches as little of it as possible. A group's colour comes
//! from its place in the sidebar, so neighbours never share a hue — which
//! hashing a name into a theme's handful of ANSI slots could not promise.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use gpui::{App, Hsla, IntoElement, Rgba, Styled, div, px};
use gpui_component::ActiveTheme as _;

use crate::core::config::Config;
use crate::ui::presets::surface_is_dark;

/// The swatch's side, and the room the header gives it before the name.
pub(crate) const SWATCH: f32 = 8.;

/// Each group turns the hue wheel this far past the one above it, so no two
/// neighbours are within 85° and the first 17 stay at least 10° apart.
const GOLDEN_ANGLE: f32 = 137.507_77;
/// The first group's hue: a blue, which no theme reads as an alarm.
const FIRST_HUE: f32 = 250.;

/// OKLCH lightness and chroma per rail: one lightness is equally bright at
/// every hue, which HSL and a theme's ANSI slots are not.
const LIGHT_RAIL: (f32, f32) = (0.60, 0.14);
const DARK_RAIL: (f32, f32) = (0.74, 0.12);

/// `#rrggbb` or `rrggbb`, in any case — the form a theme file writes.
pub(crate) fn parse_hex(s: &str) -> Option<Rgba> {
    let digits = s.strip_prefix('#').unwrap_or(s);
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(gpui::rgb)
}

/// The hue of the group drawn `slot`th in the sidebar.
pub(crate) fn slot_hue(slot: usize) -> f32 {
    (FIRST_HUE + slot as f32 * GOLDEN_ANGLE) % 360.
}

fn oklch(l: f32, c: f32, hue: f32) -> Rgba {
    let (a, b) = (c * hue.to_radians().cos(), c * hue.to_radians().sin());
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_35 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let gamma = |x: f32| {
        let x = x.clamp(0., 1.);
        match x <= 0.003_130_8 {
            true => 12.92 * x,
            false => 1.055 * x.powf(1. / 2.4) - 0.055,
        }
    };
    Rgba {
        r: gamma(4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_93 * s_),
        g: gamma(-1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_4 * s_),
        b: gamma(-0.004_196_086 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_),
        a: 1.,
    }
}

/// The override when it parses, else the hue of the group's place in the
/// sidebar (`slot`); Ungrouped (`None`) is a grey at the same lightness. A bad
/// override is logged once per group: this runs on every sidebar paint.
pub(crate) fn group_color(
    name: &str,
    slot: Option<usize>,
    overrides: &HashMap<String, String>,
    dark: bool,
) -> Hsla {
    if let Some(value) = overrides.get(name) {
        match parse_hex(value) {
            Some(c) => return c.into(),
            None => {
                static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
                let mut warned = WARNED.get_or_init(Default::default).lock().unwrap();
                if warned.insert(name.to_string()) {
                    log::warn!("group_colors[{name:?}] = {value:?} is not #rrggbb; ignoring it");
                }
            }
        }
    }
    let (l, c) = if dark { DARK_RAIL } else { LIGHT_RAIL };
    match slot {
        Some(slot) => oklch(l, c, slot_hue(slot)),
        None => oklch(l, 0., 0.),
    }
    .into()
}

pub(crate) fn dark_rail(cx: &App) -> bool {
    surface_is_dark(cx.theme().background)
}

/// The dot before a group header's name.
pub(crate) fn swatch(name: &str, slot: Option<usize>, cx: &App) -> impl IntoElement {
    let color = group_color(
        name,
        slot,
        &cx.global::<Config>().fork.group_colors,
        dark_rail(cx),
    );
    div()
        .flex_shrink_0()
        .size(px(SWATCH))
        .rounded_full()
        .bg(color)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::presets::{contrast, mix};

    /// The sidebar that asked for this: ten of these were one red.
    const GROUPS: [&str; 17] = [
        "Adv360-Pro-ZMK",
        "pixkidz",
        "infrastructure",
        "tsmux",
        "cc-statusline",
        "homelab-gitops",
        "skillz",
        "Clara_V1",
        "clawmux",
        "fnox.git",
        "tty7",
        "dotfiles",
        "clara",
        "supports-hyperlinks",
        "tty7-resume",
        "notes",
        "ssh: prod",
    ];

    fn hex(c: Hsla) -> u32 {
        let c = Rgba::from(c);
        let byte = |v: f32| (v * 255.).round() as u32;
        byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b)
    }

    fn apart(a: f32, b: f32) -> f32 {
        let d = (a - b).rem_euclid(360.);
        d.min(360. - d)
    }

    #[test]
    fn seventeen_groups_get_seventeen_hues_ten_degrees_apart_and_neighbours_far_apart() {
        let none = HashMap::new();
        for (i, a) in GROUPS.iter().enumerate() {
            for (j, b) in GROUPS.iter().enumerate().skip(i + 1) {
                let gap = apart(slot_hue(i), slot_hue(j));
                assert!(gap >= 10., "{a} and {b}: {gap}°");
            }
            if i > 0 {
                assert!(apart(slot_hue(i), slot_hue(i - 1)) >= 120., "{a}");
            }
        }
        for dark in [false, true] {
            let colours: HashSet<u32> = GROUPS
                .iter()
                .enumerate()
                .map(|(i, n)| hex(group_color(n, Some(i), &none, dark)))
                .collect();
            assert_eq!(colours.len(), GROUPS.len(), "dark={dark}");
        }
    }

    #[test]
    fn a_slot_is_the_same_colour_every_time() {
        let none = HashMap::new();
        assert_eq!(slot_hue(0), FIRST_HUE);
        assert_eq!(
            hex(group_color("tty7", Some(3), &none, false)),
            hex(group_color("other", Some(3), &none, false))
        );
    }

    #[test]
    fn ungrouped_is_a_grey() {
        for dark in [false, true] {
            let c = hex(group_color("Ungrouped", None, &HashMap::new(), dark));
            let (r, g, b) = (c >> 16 & 0xff, c >> 8 & 0xff, c & 0xff);
            assert!(r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1, "{c:06x}");
        }
    }

    /// The header's text keeps 4:1, and three quarters of what it has on the bare
    /// rail, on the wash every hue lays under it, on Solarized light and dark.
    #[test]
    fn header_text_stays_readable_on_every_fill() {
        let none = HashMap::new();
        for (dark, rail, text) in [(false, 0xfdf6e3, 0x586e75), (true, 0x002b36, 0x93a1a1)] {
            for slot in (0..GROUPS.len()).map(Some).chain([None]) {
                let c = hex(group_color("g", slot, &none, dark));
                let fill = mix(rail, c, crate::ui::group_header::FILL_ALPHA);
                let ratio = contrast(text, fill);
                assert!(
                    ratio >= 4. && ratio >= 0.75 * contrast(text, rail),
                    "dark={dark} slot={slot:?}: {ratio}"
                );
                // The swatch reads as a dot against the rail.
                assert!(contrast(c, rail) >= 2., "dark={dark} slot={slot:?}");
            }
        }
    }

    #[test]
    fn a_valid_override_wins_and_a_bad_one_falls_back() {
        for slot in [Some(2), None] {
            let derived = hex(group_color("tty7", slot, &HashMap::new(), false));
            for (value, want) in [
                ("#ff8800", 0xff8800),
                ("00AAff", 0x00aaff),
                ("#12345", derived),
                ("#gg0000", derived),
                ("", derived),
            ] {
                let overrides = HashMap::from([("tty7".to_string(), value.to_string())]);
                assert_eq!(
                    hex(group_color("tty7", slot, &overrides, false)),
                    want,
                    "{value:?}"
                );
            }
        }
    }

    #[test]
    fn hex_parsing_takes_six_digits_with_or_without_a_hash_in_any_case() {
        for (input, want) in [
            ("#000000", Some(0x000000)),
            ("ffffff", Some(0xffffff)),
            ("#AbCdEf", Some(0xabcdef)),
            ("#abc", None),
            ("#1234567", None),
            ("##123456", None),
            ("12345g", None),
            ("#+12345", None),
            (" 123456", None),
            ("", None),
            ("#", None),
            ("éé1234", None),
        ] {
            let got = parse_hex(input).map(|c| {
                let byte = |v: f32| (v * 255.).round() as u32;
                byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b)
            });
            assert_eq!(got, want, "{input:?}");
        }
    }
}
