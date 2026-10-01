# OSC 8 links rest under a faint dotted underline

`Fork-Feature: osc8-underline`. The commit that carries this feature is `fork(osc8-underline): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/terminal/element/osc8_underline.rs` | an OSC 8 link's resting faint dotted underline (iTerm2's), solid under the pointer; an SGR underline keeps its own |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/terminal/element.rs` | `mod osc8_underline`, `RenderCell::osc8_dots`, end of `snapshot_cell`, `flag_hovered_link`; test `test_colors` made `pub(super)` | `osc8_underline::mark` / `unmark` |
