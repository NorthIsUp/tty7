# T2 — Group colours (F2)

Read `docs/fork/spec.md` §F2 and `docs/fork/master-plan.md` first.

- New `src/ui/group_color.rs` (and `pub mod group_color;` in `src/ui/mod.rs`):
  - `pub(crate) fn fnv1a(s: &str) -> u64`;
  - `pub(crate) fn parse_hex(s: &str) -> Option<gpui::Rgba>`, which accepts
    `#rrggbb` and `rrggbb` in any case;
  - `pub(crate) fn group_color(name: &str, overrides: &HashMap<String,String>, ansi16: &[(u8,u8,u8);16]) -> gpui::Hsla`.
    It uses the override when it parses (log an invalid one once per name via
    a `OnceLock<Mutex<HashSet>>`), else `ansi16[PALETTE[fnv1a(name) % 12]]`
    with `PALETTE = [1,2,3,4,5,6,9,10,11,12,13,14]`.
- Hook: in `src/ui/tab_sidebar.rs`, in the group header row (next to
  `sidebar-group-name`, ~:1375), prepend an 8×8 rounded swatch
  (`div().size(px(8.)).rounded_full().bg(color)`). Get `ansi16` from the active
  preset: find how `presets.rs` exposes the current `ansi16`, and if there is
  no accessor add a tiny one in group_color.rs rather than in presets.rs.
  Keep the hook to a few lines and list it for `FORK.md`.
- Tests: the same name gives the same colour; different names are spread
  (≥ 6 distinct colours across 12 sample names); the override wins; a bad
  override falls back; hex parsing covers the edge cases.
- Docs: a short "Group colours" section in `docs/window/sidebar.mdx`.
