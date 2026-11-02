# a hidden pane spawns at the window's real grid, not 80x24

`Fork-Feature: spawn-grid`. The commit that carries this feature is `fork(spawn-grid): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/terminal/remote/fork_pane.rs` | `RemoteTerminal::laid_out_grid`: the grid and cell a pane last asked for, once a layout sized it; what `background_grid` spawns new panes at |
| `src/ui/background_grid.rs` | the grid new panes spawn at: `WholeTabGrid`, each window's last plain single-pane terminal grid (not while a document column is docked), `spawn_size` (80×24 before the window has one), `track`, which, once the grid has held 200ms, hands it to every never-shown single-pane tab not at it yet (adopted, or spawned at an older grid), so an agent in a background tab is already the size it is shown at |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/app.rs`, `src/ui/tree_sync.rs`, `src/ui/pending_pane.rs`, `src/terminal/view.rs` | `Tty7App::whole_tab_grid`, `background_grid::track` at the top of `Render for Tty7App`; a `grid` argument through `tabs_from_session`, `new_terminal`, `new_terminal_native`, `session_to_pane`, `PendingSpawn` and `start_pane_spawn` into `spawn_shell_terminal_in` / `spawn_native_ssh_terminal`, in place of their 80×24 | a pane spawns at its window's whole-tab grid, not 80×24 |
