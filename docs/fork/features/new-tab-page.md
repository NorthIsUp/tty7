# New Tab opens a picker for what to open and where

`Fork-Feature: new-tab-page`. The commit that carries this feature is `fork(new-tab-page): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `docs/window/new-tab-page.mdx` | user docs for the new tab page |
| `src/ui/new_tab_page.rs` | the new tab page picker (agent or terminal, and a directory), drawn as Search Everywhere's New Tab tab; `open_palette_on`, what ⌘T/⌘P/⌘K do (open, switch tab, or close) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/fork_config.rs` | `new_tab_hidden_agents` | the agents the page leaves out |
| `src/ui/settings/agents.rs` | `render_new_tab_agents_rows` | Agents in New Tab: a switch per agent on `PATH` |
| `src/ui/settings.rs`, `src/ui/app.rs` | search entries, config key, description, modified check; reset | Agents in New Tab as a setting like any other |
| `src/ui/i18n/{mod,en,ja,ru,zh}.rs` | after `GitHubOpenInGraphite` | `SettingsNewTabAgents`(`Desc`), `SettingsSearchNewTabAgentsKeywords` |
| `src/ui/app.rs` | `new_tab` | open Search Everywhere's New Tab tab when `new_tab_page` is on |
| `src/ui/tab_sidebar.rs` | `new_tab_in_group` | `new_tab_with_shell(None, ..)` so a group's New Tab skips the page |
| `src/ui/palette.rs` | module doc, imports; `chord_tab` (was `is_palette_chord`) and its two call sites, the new tab page's keys in `intercept`, then `SearchView::on_arrow` (+ tests); `recording_a_shortcut` is upstream's, unchanged | upstream's since #1026, for ⌘P; the fork adds ⌘T (New Tab) and ⌘K (Sessions) as tabs the same chord logic opens, and the new tab page's own keys ahead of the modal rule |
| `src/ui/search/mod.rs` | `SearchTab`, `ORDER`, `title`, `placeholder`, module list, `CARD_MAX_W` re-export (+ test) | the `Text` tab, on the row between Hosts and Actions; the `History` tab, after Sessions; the `NewTab` tab, off the row |
| `src/ui/search/view.rs` | `SearchView::new_tab`, `set_new_tab`, `new_tab_page`, `focus`; `render` card; `CARD_MAX_W` `pub(crate)`; the card's parts as free functions (`card_top`, `list_max_h`, `hit`, `title_ink`, `esc_cap`, `scope_bar`, `scope_chip`, `footer_bar`, `footer_hint`), `SearchRow`'s fields and a few sizes `pub(crate)`; `mod view` `pub(crate)` in `search/mod.rs` | the New Tab tab draws `NewTabPage` in place of the list, built from the palette's own parts so the two cards look the same; `palette` puts focus back in the field |
| `docs/window/search-everywhere.mdx` | intro; Tabs table; Sessions | ⌘T/⌘P/⌘K from anywhere, modal; the Text, History and New Tab tabs; ⌘K opens Sessions; ⇧ opens in the background |
| `docs/reference/keyboard-shortcuts.mdx` | New Tab, Search Everywhere, Clear Scrollback rows; after View | ⌘K is Sessions; Clear Scrollback moved to ⌘⇧K; ⌘T is the palette's New Tab tab; the three chords work from anywhere and the palette is modal |
| `docs/docs.json` | "The window" pages | `window/new-tab-page`, `window/hotkey-window` |
