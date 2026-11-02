# ⇧ opens a tab in the background

`Fork-Feature: background-tabs`. The commit that carries this feature is `fork(background-tabs): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/background_tab.rs` | ⇧ opens a tab in the background: `in_background`, `maybe_background` (when ⇧ is held), `seat_new_tab`, which palette rows take it, where `active` lands; `bind_shift_enter`, the palette list's ⇧Enter |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/keymap.rs` | `init`; `fixed_bindings` ⌘K ⌘D comment | `background_tab::bind_shift_enter` first, so the base snapshot keeps it; the palette takes ⌘K first on macOS |
| `src/ui/app.rs` | `Tty7App::open_in_background` field + init; `new_tab_slot` → `seat_new_tab`; `new_tab_insert_at` made `pub(crate)` | insert without activating inside `in_background` |
| `src/ui/app.rs` | `run_command` `LaunchAgent`, `ResumeSession`, `ForkSession` | wrap in `maybe_background` (⇧ held → `in_background`) |
| `src/ui/search/view.rs` | `render_footer` | the ⇧↵ footer hint on tab-opening rows |
