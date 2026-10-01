# ⇧ opens a tab in the background

`Fork-Feature: background-tabs`. The commit that carries this feature is `fork(background-tabs): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/background_grid.rs` | the grid new panes spawn at: `WholeTabGrid`, each window's last plain single-pane terminal grid (not while a document column is docked), `spawn_size` (80×24 before the window has one), `track`, which, once the grid has held 200ms, hands it to every never-shown single-pane tab not at it yet (adopted, or spawned at an older grid), so an agent in a background tab is already the size it is shown at |
| `src/ui/background_tab.rs` | ⇧ opens a tab in the background: `in_background`, `maybe_background` (when ⇧ is held), `seat_new_tab`, which palette rows take it, where `active` lands; `bind_shift_enter`, the palette list's ⇧Enter |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/app.rs`, `src/ui/tree_sync.rs`, `src/ui/pending_pane.rs`, `src/terminal/view.rs` | `Tty7App::whole_tab_grid`, `background_grid::track` at the top of `Render for Tty7App`; a `grid` argument through `tabs_from_session`, `new_terminal`, `new_terminal_native`, `session_to_pane`, `PendingSpawn` and `start_pane_spawn` into `spawn_shell_terminal_in` / `spawn_native_ssh_terminal`, in place of their 80×24 | a pane spawns at its window's whole-tab grid, not 80×24 |
| `src/ui/keymap.rs` | `init`; `fixed_bindings` ⌘K ⌘D comment | `background_tab::bind_shift_enter` first, so the base snapshot keeps it; the palette takes ⌘K first on macOS |
| `src/ui/app.rs` | `Tty7App::open_in_background` field + init; `new_tab_slot` → `seat_new_tab`; `new_tab_insert_at` made `pub(crate)` | insert without activating inside `in_background` |
| `src/ui/app.rs` | `run_command` `LaunchAgent`, `ResumeSession`, `ForkSession` | wrap in `maybe_background` (⇧ held → `in_background`) |
| `src/ui/search/view.rs` | `render_footer` | the ⇧↵ footer hint on tab-opening rows |
