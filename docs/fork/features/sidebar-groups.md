# group colours, outline and fill, animated fold, default branch in the header

`Fork-Feature: sidebar-groups`. The commit that carries this feature is `fork(sidebar-groups): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/group_color.rs` | a group's colour (override, else a golden-angle hue by sidebar place; Ungrouped grey) |
| `src/ui/group_header.rs` | `Deco`, one group's decoration per frame (the outline round the whole group, the fill on the header or whole group, swatch, chevron, the fold slide and its clip), fold states forgotten a minute after their group or window stops drawing, the repo default branch it names, and their Settings rows; `pin_clicked`, the pin's click (unpin a folder group, delete a label group after a confirm) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/tab_sidebar.rs` | header pin mark (clickable on label groups too, `debug_selector`, click → `group_header`'s `pin_clicked`), tests `clicking_a_folder_groups_pin_unpins_it`, `clicking_a_label_groups_pin_deletes_it_once_confirmed` | a click on a folder group's pin unpins it; on a label group's it deletes the group, name and hand-picked members, after a confirm. The label-group click (without the confirm) is offered upstream as `upstream/label-group-unpin` |
| `crates/tty7-core/src/core/group_key.rs` | `WorkspaceGroups::held` | the pinned tabs, stored with the groups so they persist in `machine.json` and every window agrees |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` section loop, after `folded` | `let deco = group_header::Deco::new(..)` once per group; then `deco.folded_away(folded)` (rows stay drawn until a fold's slide ends), `rows_h` summed per row, the header's width budget for the chevron and swatch, `deco.bar`, `deco.chevron()` on every header, `deco.swatch()`, `deco.backing()` under the hover buttons, `deco.block`, `deco.clip(rows, ..)` |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` section loop, `header_git` after `shared_git`; `SharedGit` `pub(crate)` | `group_header::header_git`: the header names the repo's default branch, not the rows' checkout |
| `src/ui/tab_sidebar.rs` | test `folding_a_group_takes_its_rows_off_the_sidebar` | `animations = false`: it checks where a fold ends, not its slide |
| `src/ui/tab_sidebar.rs` | `row_metrics::header_budget` doc | the chevron is always drawn now |
| `src/ui/settings/pages.rs` | `render_settings_appearance` | a Tabs group of `group_header_settings` rows, before Window |
| `docs/window/sidebar.mdx` | Group colours | `group_colors`, header outline/fill, default branch |
| `src/ui/tab_sidebar.rs` | after `Deco::new`, `sf.selected_on` of `deco.rows_under()`, when a wash covers the rows, replaces `sf.selected` / `sf.text_selected` on the active row; active title `FontWeight::BOLD` | the current tab steps off the group's wash, not the rail, so it stays visible inside a filled group |
| `src/ui/presets.rs` | `Surface::fg`, `Surface::selected_on` (+ test) | a selected row on a group's wash steps off the wash, not the rail |
