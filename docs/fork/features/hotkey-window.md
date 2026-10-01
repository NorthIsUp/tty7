# a global hotkey window

`Fork-Feature: hotkey-window`. The commit that carries this feature is `fork(hotkey-window): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/hotkey_window/` | the global hotkey (`global_hotkey`, ⌥Space), macOS only: `mod.rs` the platform-free rules (toggle, the lift stack, restore, the switcher row's item and badge, the Workspaces menu label); `carbon.rs` the chord table and `RegisterEventHotKey`; `appkit.rs` the one dedicated hotkey window (its workspace saved in `hotkey-window`) shown / focused / ordered out with a fade, full screen (the screen below its menu bar, floating level under Force Quit, the Dock moved aside by presentation options while it is the key window and covered, and only what it set put back), windows activated over it lifted above it, hide on focus loss; `settings.rs` its Settings rows |
| `docs/window/hotkey-window.mdx` | user docs for the hotkey window |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/main.rs` | `main`, after `keymap::init` | `hotkey_window::init` |
| `src/ui/app.rs` | `with_session_at`'s `observe_window_bounds`; `window_bounds` made `pub(crate)` for its test | skip `hotkey_window::covering`, so the full screen frame never becomes the workspace's saved geometry |
| `src/ui/settings/pages.rs` | `render_settings_appearance`, after the window section | `hotkey_window_settings` rows |
| `Cargo.toml` | macOS deps | `raw-window-handle`, for the hotkey window's NSWindow; `block2`, for its AppKit notification observers |
| `src/ui/windows.rs` | `WindowRegistry::most_recent`, `most_recent_local` | skip `hotkey_window::workspace`, so the Dock, the tray and the CLI never land in the hotkey window |
| `src/core/session.rs` | `WorkspaceStore::restore_one` | `hotkey_window::to_restore`: a launch or a Dock click never reopens the hotkey window as a plain one |
| `src/ui/switcher.rs` | `row_menu` (one line after the workspace verbs), `render_row` (after the slot number) | `hotkey_window::menu_item` (Set as / Unset Hotkey Workspace) and `row_badge` (the chord's keycaps on the hotkey workspace's row) |
| `src/ui/theme.rs` | `window_menu_items` | `hotkey_window::menu_label`: the chord after the hotkey workspace in the Workspaces menu |
