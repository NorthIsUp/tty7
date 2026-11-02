# a pinned folder group holds the tabs whose cwd is in it; a tab can be held

`Fork-Feature: group-follow`. The commit that carries this feature is `fork(group-follow): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/group_follow.rs` | folder-group membership follows the cwd, overriding upstream's "a tab in one only ever leaves by hand" (`group_key.rs`) for folder groups: `walked_out` (a tab not pinned to its folder group leaves once its answered cwd is outside the folder, after a restart too), `inherits` (⌘T inherits only a label group); `held`, `hold_tab_group`, `hold_item`: a tab pinned to its current group (Pin to Current Group, or put in a group by hand: drag, Move to Group) neither walks out nor in and keeps its auto group (+ tests) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/tab_sidebar.rs` | `set_tab_group` and `move_tab_to` (`hold_tab_group`), `tab_auto_group` and `settle_sidebar_groups` (skip a `held` tab) | a tab put in a group by hand (drag, Move to Group) is pinned there, and one handed back to auto grouping unpinned; a pinned tab is not regrouped |
| `src/ui/tab_strip.rs` | `tab_context_menu`, after Move to Group | `group_follow::hold_item`, the Pin to Current Group toggle |
| `src/ui/i18n/{mod,en,ja,ru,zh}.rs` | `SidebarPinToCurrentGroup` | the toggle's label |
| `src/ui/tab_sidebar.rs` | `settle_sidebar_groups` (`walked_out`), `spawn_group` (`inherits`); `fold_tests` helpers re-exported `pub(crate)`, `SpawnPlace::group` `pub(crate)`, for `group_follow`'s tests | a tab sits in a folder group only while its cwd is in the folder, or the user put it there |
| `crates/tty7-core/src/core/group_key.rs` | `pinned_folder_for` breaks an equal-depth tie for the folder the cwd is in (+ test) | header order never decides a worktree's group |
