# New Tab asks what to open and where

`Fork-Feature: palette-tabs`. The commit that carries this feature is `fork(palette-tabs): …` on `main-niu`.

New Tab opens a picker for what to open and where, drawn as Search Everywhere's New Tab tab, and ⌘D/⌘⇧D open it to pick a split's harness and directory, as `split_harness` and `split_dir` say (`choose` by default, starting on the split pane's), or split straight away when neither asks; Search Everywhere gains Text and History tabs, and ⌘K opens Sessions with open sessions first. One feature because the New Tab tab and the Text, History and Agents tabs share `SearchTab`, the palette's chords and its card.

## Fork-owned files

| file | what it holds |
|---|---|
| `docs/window/new-tab-page.mdx` | user docs for the new tab page |
| `src/ui/new_tab_page.rs` | the new tab page picker (agent or terminal, and a directory), drawn as Search Everywhere's New Tab tab; `open_palette_on`, what ⌘T/⌘P/⌘K do (open, switch tab, or close) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/fork_config.rs` | `new_tab_hidden_agents` | the agents the page leaves out |
| `src/ui/settings/agents.rs` | `new_tab_cell`, `render_new_tab_only_row`, `render_split_rows`; `render_agent_hooks_group` (header, `has`, "more agents" count), `render_agent_row` (`new_tab` cell) | In New Tab: a column of the local integrations list, a switch per agent on `PATH`; an agent on `PATH` with no hooks gets a row of its own; agents on `PATH` are listed without Show all |
| `src/ui/settings.rs`, `src/ui/app.rs` | search entries, config key, description, modified check; reset | In New Tab as a setting like any other |
| `src/ui/i18n/{mod,en,ja,ru,zh}.rs` | after `GitHubOpenInGraphite` | `SettingsNewTabAgents`(`Desc`), `SettingsAgentNoHooks`, `SettingsSearchNewTabAgentsKeywords`; `SettingsSplitHarness`(`Desc`), `SettingsSplitDir`(`Desc`), their `SettingsSearch…Keywords`, `SettingsSplitTerminal`/`Same`/`Choose` |
| `src/ui/app.rs` | `new_tab` | open Search Everywhere's New Tab tab when `new_tab_page` is on |
| `src/ui/app.rs` | `split`, `split_slot_in` (`split_slot` with a start directory) | ⌘D/⌘⇧D go through `open_split_page`: the page with a split as its target, or the split per `split_harness` and `split_dir` |
| `src/ui/agent_launch.rs` | `split_agent_in`, which `launch_agent`'s split also goes through | an agent picked for a split starts in the pane the split placed, in the picked directory |
| `src/ui/app.rs` | `focused_leaf` `pub(crate)` | `open_split_page` asks whether the focused pane is remote, and what agent it runs |
| `crates/tty7-core/src/core/fork_config.rs` | `split_harness`, `split_dir`, `SplitHarness`, `SplitDir`; `split_page` read and dropped by `migrate` | the split picker's settings; `"split_page": false` becomes `terminal` + `same` |
| `crates/tty7-core/src/core/config.rs` | `sanitize`, first line | `fork.migrate()` |
| `src/ui/settings/agents.rs`, `src/ui/settings.rs`, `src/ui/app.rs` | the Split runs and Split directory rows; search entries, config keys, descriptions, modified checks, resets | settings like any other |
| `src/ui/tab_sidebar.rs` | `new_tab_in_group` | `new_tab_with_shell(None, ..)` so a group's New Tab skips the page |
| `src/ui/palette.rs` | module doc, imports; `chord_tab` (was `is_palette_chord`) and its two call sites, the new tab page's keys in `intercept`, then `SearchView::on_arrow` (+ tests); `recording_a_shortcut` is upstream's, unchanged | upstream's since #1026, for ⌘P; the fork adds ⌘T (New Tab) and ⌘K (Sessions) as tabs the same chord logic opens, and the new tab page's own keys ahead of the modal rule |
| `src/ui/search/view.rs` | `SearchView::new_tab`, `set_new_tab`, `new_tab_page`, `focus`; `render` card; `CARD_MAX_W` `pub(crate)`; the card's parts as free functions (`section_header`, `card_top`, `list_max_h`, `hit`, `title_ink`, `esc_cap`, `scope_bar`, `scope_chip`, `footer_bar`, `footer_hint`), `SearchRow`'s fields and a few sizes `pub(crate)`; `mod view` `pub(crate)` in `search/mod.rs` | the New Tab tab draws `NewTabPage` in place of the list, built from the palette's own parts so the two cards look the same; `palette` puts focus back in the field; `a_query_after_one_that_found_nothing_still_arms_its_first_row` sets upstream's split, which its Split Right runs |
| `docs/window/search-everywhere.mdx` | intro; Tabs table; Sessions | ⌘T/⌘P/⌘K from anywhere, modal; the Text, History and New Tab tabs; ⌘K opens Sessions; ⇧ opens in the background |
| `docs/reference/keyboard-shortcuts.mdx` | New Tab, Search Everywhere, Clear Scrollback rows; after View | ⌘K is Sessions; Clear Scrollback moved to ⌘⇧K; ⌘T is the palette's New Tab tab; the three chords work from anywhere and the palette is modal |
| `docs/window/tabs-and-splits.mdx` | Splits | ⌘D/⌘⇧D ask first; links the new tab page |
| `docs/docs.json` | "The window" pages | `window/new-tab-page`, `window/hotkey-window` |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::SearchText` |
