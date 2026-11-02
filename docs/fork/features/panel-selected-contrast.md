# the selected sidebar and panel row stands further off its background

`Fork-Feature: panel-selected-contrast`. The commit that carries this feature is `fork(panel-selected-contrast): …` on `main-niu`.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/presets.rs` | `panel_selected` (shared with `panel_surface`), `PANEL_SELECTED` 1.12 → 1.22 | the selected rung measured from the fill under the row, and a stronger rung |
