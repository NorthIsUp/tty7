//! The fork's sidebar group colours, kept out of `tab_sidebar.rs` so rebasing
//! on upstream touches as little of it as possible. A group's colour is
//! derived from its name, so the same group looks the same in every window
//! and across restarts without anything being stored.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use gpui::{App, Hsla, IntoElement, Rgba, Styled, div, px};

use crate::core::config::Config;
use crate::terminal::palette::ActivePalette;
use crate::ui::{presets, theme};

/// The swatch's side, and the room the header gives it before the name.
pub(crate) const SWATCH: f32 = 8.;

/// The theme's six hues in both halves, skipping black, white and the greys
/// (0, 7, 8, 15): a grey swatch reads as no colour at all.
const PALETTE: [usize; 12] = [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14];

pub(crate) fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// `#rrggbb` or `rrggbb`, in any case — the form a theme file writes.
pub(crate) fn parse_hex(s: &str) -> Option<Rgba> {
    let digits = s.strip_prefix('#').unwrap_or(s);
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(gpui::rgb)
}

/// The override when it parses, else the hue the name hashes to. A bad
/// override is logged once per group: this runs on every sidebar paint.
pub(crate) fn group_color(
    name: &str,
    overrides: &HashMap<String, String>,
    ansi16: &[(u8, u8, u8); 16],
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
    let (r, g, b) = ansi16[PALETTE[(fnv1a(name) % PALETTE.len() as u64) as usize]];
    gpui::rgb(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)).into()
}

/// The terminal palette the theme applied, which the sidebar has no handle
/// on otherwise; before the first apply, the configured preset's own.
fn active_ansi16(cx: &App) -> [(u8, u8, u8); 16] {
    match cx.try_global::<ActivePalette>() {
        Some(p) => p.ansi16.map(|c| (c.r, c.g, c.b)),
        None => presets::by_id(cx, &theme::effective_preset_id(cx)).ansi16,
    }
}

/// The dot before a group header's name.
pub(crate) fn swatch(name: &str, cx: &App) -> impl IntoElement {
    let color = group_color(
        name,
        &cx.global::<Config>().group_colors,
        &active_ansi16(cx),
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

    const ANSI16: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (16, 0, 0),
        (32, 0, 0),
        (48, 0, 0),
        (64, 0, 0),
        (80, 0, 0),
        (96, 0, 0),
        (112, 0, 0),
        (128, 0, 0),
        (144, 0, 0),
        (160, 0, 0),
        (176, 0, 0),
        (192, 0, 0),
        (208, 0, 0),
        (224, 0, 0),
        (240, 0, 0),
    ];

    fn hex(c: Hsla) -> u32 {
        let c = Rgba::from(c);
        let byte = |v: f32| (v * 255.).round() as u32;
        byte(c.r) << 16 | byte(c.g) << 8 | byte(c.b)
    }

    #[test]
    fn the_same_name_gets_the_same_colour() {
        let none = HashMap::new();
        assert_eq!(
            hex(group_color("tty7", &none, &ANSI16)),
            hex(group_color("tty7", &none, &ANSI16))
        );
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn different_names_spread_over_at_least_six_colours() {
        let none = HashMap::new();
        let names = [
            "tty7",
            "clawmux",
            "dotfiles",
            "clara",
            "infra",
            "web",
            "api",
            "docs",
            "scratch",
            "notes",
            "ssh: prod",
            "Ungrouped",
        ];
        let colours: HashSet<u32> = names
            .iter()
            .map(|n| hex(group_color(n, &none, &ANSI16)))
            .collect();
        assert!(colours.len() >= 6, "only {} colours", colours.len());
        let greys: HashSet<u32> = [0, 7, 8, 15].iter().map(|&i| (i * 16) << 16).collect();
        assert!(colours.is_disjoint(&greys), "a group landed on a grey");
    }

    #[test]
    fn a_valid_override_wins_and_a_bad_one_falls_back() {
        let derived = hex(group_color("tty7", &HashMap::new(), &ANSI16));
        for (value, want) in [
            ("#ff8800", 0xff8800),
            ("00AAff", 0x00aaff),
            ("#12345", derived),
            ("#gg0000", derived),
            ("", derived),
        ] {
            let overrides = HashMap::from([("tty7".to_string(), value.to_string())]);
            assert_eq!(
                hex(group_color("tty7", &overrides, &ANSI16)),
                want,
                "{value:?}"
            );
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
