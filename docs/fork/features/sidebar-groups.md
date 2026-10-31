# group colours, headers, fill, fold and tab pins

`Fork-Feature: sidebar-groups`. The commit that carries this feature is `fork(sidebar-groups): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/group_color.rs` | a group's colour (override, else a golden-angle hue by sidebar place; Ungrouped grey) |
| `src/ui/group_follow.rs` | folder-group membership follows the cwd, overriding upstream's "a tab in one only ever leaves by hand" (`group_key.rs`) for folder groups: `walked_out` (a tab not pinned to its folder group leaves once its answered cwd is outside the folder, after a restart too), `inherits` (⌘T inherits only a label group); `held`, `hold_tab_group`, `hold_item`: a tab pinned to its current group (Pin to Current Group, or put in a group by hand: drag, Move to Group) neither walks out nor in and keeps its auto group (+ tests) |
| `crates/tty7-core/src/core/group_migrate.rs` | `run_once`: a one-shot repair of tabs an older build misfiled in folder groups, marked done by `group-membership-repaired` beside `machine.json` once the tree is written (a fresh install with its first save, a failed load never): the live rule on each tab's last cwd and repo, symlinks resolved and unresolved paths compared as spelled too; with no cwd, a tab whose repo holds its folder or sits under another pinned one is left alone; a tab whose paths do not resolve is skipped (+ tests) |
| `src/ui/group_header.rs` | `Deco`, one group's decoration per frame (the outline round the whole group, the fill on the header or whole group, swatch, chevron, the fold slide and its clip), fold states forgotten a minute after their group or window stops drawing, the repo default branch it names, and their Settings rows; `pin_clicked`, the pin's click (unpin a folder group, delete a label group after a confirm) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/tab_sidebar.rs` | header pin mark (clickable on label groups too, `debug_selector`, click → `group_header`'s `pin_clicked`), tests `clicking_a_folder_groups_pin_unpins_it`, `clicking_a_label_groups_pin_deletes_it_once_confirmed` | a click on a folder group's pin unpins it; on a label group's it deletes the group, name and hand-picked members, after a confirm. The label-group click (without the confirm) is offered upstream as `upstream/label-group-unpin` |
| `src/ui/tab_sidebar.rs` | `set_tab_group` and `move_tab_to` (`hold_tab_group`), `tab_auto_group` and `settle_sidebar_groups` (skip a `held` tab) | a tab put in a group by hand (drag, Move to Group) is pinned there, and one handed back to auto grouping unpinned; a pinned tab is not regrouped |
| `crates/tty7-core/src/core/group_key.rs` | `WorkspaceGroups::held` | the pinned tabs, stored with the groups so they persist in `machine.json` and every window agrees |
| `src/ui/tab_strip.rs` | `tab_context_menu`, after Move to Group | `group_follow::hold_item`, the Pin to Current Group toggle |
| `src/ui/i18n/{mod,en,ja,ru,zh}.rs` | `SidebarPinToCurrentGroup` | the toggle's label |
| `src/ui/tab_sidebar.rs` | `settle_sidebar_groups` (`walked_out`), `spawn_group` (`inherits`); `fold_tests` helpers re-exported `pub(crate)`, `SpawnPlace::group` `pub(crate)`, for `group_follow`'s tests | a tab sits in a folder group only while its cwd is in the folder, or the user put it there |
| `crates/tty7-core/src/core/machine.rs` | `MachineStore::open` calls `group_migrate::run_once` (an empty tree re-parsed to tell it from a failed load) and flushes; `mark_repaired`, and `persist` writes the marker after the tree | the one-shot repair runs on the tree every window reads, and is recorded only once that tree is on disk |
| `crates/tty7-core/src/core/group_key.rs` | `pinned_folder_for` breaks an equal-depth tie for the folder the cwd is in (+ test) | header order never decides a worktree's group |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` section loop, after `folded` | `let deco = group_header::Deco::new(..)` once per group; then `deco.folded_away(folded)` (rows stay drawn until a fold's slide ends), `rows_h` summed per row, the header's width budget for the chevron and swatch, `deco.bar`, `deco.chevron()` on every header, `deco.swatch()`, `deco.backing()` under the hover buttons, `deco.block`, `deco.clip(rows, ..)` |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` section loop, `header_git` after `shared_git`; `SharedGit` `pub(crate)` | `group_header::header_git`: the header names the repo's default branch, not the rows' checkout |
| `src/ui/tab_sidebar.rs` | test `folding_a_group_takes_its_rows_off_the_sidebar` | `animations = false`: it checks where a fold ends, not its slide |
| `src/ui/tab_sidebar.rs` | `row_metrics::header_budget` doc | the chevron is always drawn now |
| `src/ui/settings/pages.rs` | `render_settings_appearance` | a Tabs group of `group_header_settings` rows, before Window |
| `docs/window/sidebar.mdx` | Group colours | `group_colors`, header outline/fill, default branch |
| `src/ui/tab_sidebar.rs` | after `Deco::new`, `sf.selected_on` of `deco.rows_under()`, when a wash covers the rows, replaces `sf.selected` / `sf.text_selected` on the active row; active title `FontWeight::BOLD` | the current tab steps off the group's wash, not the rail, so it stays visible inside a filled group |
| `src/ui/presets.rs` | `Surface::fg`, `Surface::selected_on`, `panel_selected` (shared with `panel_surface`), `PANEL_SELECTED` 1.12 → 1.22 (+ test) | the selected rung measured from the fill under the row, and a stronger rung |
