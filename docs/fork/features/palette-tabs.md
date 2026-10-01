# The New Tab picker and Search Everywhere's tabs

`Fork-Feature: palette-tabs`. The commit that carries this feature is `fork(palette-tabs): …` on `main-niu`.

New Tab opens a picker for what to open and where, drawn as Search Everywhere's New Tab tab; Search Everywhere gains Text and History tabs, and ⌘K opens Sessions with open sessions first. One feature because the New Tab tab and the Text, History and Agents tabs share `SearchTab`, the palette's chords and its card.

## Fork-owned files

| file | what it holds |
|---|---|
| `docs/window/new-tab-page.mdx` | user docs for the new tab page |
| `src/ui/new_tab_page.rs` | the new tab page picker (agent or terminal, and a directory), drawn as Search Everywhere's New Tab tab; `open_palette_on`, what ⌘T/⌘P/⌘K do (open, switch tab, or close) |
| `src/ui/search/text.rs` | Search Everywhere's Text tab: find in files over `Host::search_content`, debounced, never on All; `LiveTab`, the per-palette ask, debounce and latest-answer check History shares |
| `src/ui/search/open_sessions.rs` | the Sessions tab's lead (⌘K): sessions open in a tab move to the front and go to that tab (`open_first`); `open_agent_sessions`, every open session with its tab |
| `src/ui/search/history_text.rs` | Search Everywhere's History tab: full text over past agent conversations, one row per session, Enter resumes it |
| `crates/tty7-core/src/core/history_search.rs` | the History tab's scan: Claude, Qoder and Codex transcripts streamed newest first, what was said cached (`history_cache`); `session_mentions`, one session's issue and PR references (`#N`, `owner/repo#N`, `/pull/N` and `/issues/N` links), tool output included; only the shown repo's, a bare `#N` only when the session ran in a checkout of it, never a colour like `#333333` or `#0` |
| `crates/tty7-core/src/core/history_cache.rs` | the History tab's transcript cache: by path, size and mtime, least recently read dropped past the cap, deleted files forgotten |

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
| `src/ui/search/view.rs` | `SearchView::new_tab`, `set_new_tab`, `new_tab_page`, `focus`; `render` card; `CARD_MAX_W` `pub(crate)`; the card's parts as free functions (`section_header`, `card_top`, `list_max_h`, `hit`, `title_ink`, `esc_cap`, `scope_bar`, `scope_chip`, `footer_bar`, `footer_hint`), `SearchRow`'s fields and a few sizes `pub(crate)`; `mod view` `pub(crate)` in `search/mod.rs` | the New Tab tab draws `NewTabPage` in place of the list, built from the palette's own parts so the two cards look the same; `palette` puts focus back in the field |
| `docs/window/search-everywhere.mdx` | intro; Tabs table; Sessions | ⌘T/⌘P/⌘K from anywhere, modal; the Text, History and New Tab tabs; ⌘K opens Sessions; ⇧ opens in the background |
| `docs/reference/keyboard-shortcuts.mdx` | New Tab, Search Everywhere, Clear Scrollback rows; after View | ⌘K is Sessions; Clear Scrollback moved to ⌘⇧K; ⌘T is the palette's New Tab tab; the three chords work from anywhere and the palette is modal |
| `docs/docs.json` | "The window" pages | `window/new-tab-page`, `window/hotkey-window` |
| `crates/tty7-core/src/core/agent_history.rs` | `Found`, `claude_files`, `codex_files`, `codex_not_the_users`, `strip_injected`, `unix` made `pub(crate)` | `history_search` walks and filters the same files |
| `src/ui/app.rs` | `search_sessions` | its rows go through `open_first` |
| `src/ui/machine_mirror.rs` | `agent_sessions_of` | the sessions open in a mirrored workspace's tabs, for `open_agent_sessions` |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `SearchText` (Search Text in Files…) |
| `src/ui/search/sources.rs` | `Catalog` fields, `new`, `source`, `all` | `live` (`text::LiveTab`s); `all` leaves Text and History out; New Tab has no rows of its own; `Sessions::highlights` leaves open sessions to the Terminals rows |
| `src/ui/search/view.rs` | `perform_search`, `set_tab`, `render_empty`, `update_catalog` visibility | `Catalog::ask_live` for the tab showing; the too-short and remote hints |
| `src/ui/panel_search.rs` | module list | `pub(crate) mod model` for `split_relative` |
| `src/ui/app.rs` | `open_search` | `catalog.live = palette_live_tabs(..)` |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::SearchText` |
