# Search Everywhere's Text and History tabs, and open sessions first

`Fork-Feature: search-tabs`. The commit that carries this feature is `fork(search-tabs): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/search/text.rs` | Search Everywhere's Text tab: find in files over `Host::search_content`, debounced, never on All; `LiveTab`, the per-palette ask, debounce and latest-answer check History shares |
| `src/ui/search/open_sessions.rs` | the Sessions tab's lead (⌘K): sessions open in a tab move to the front and go to that tab (`open_first`); `open_agent_sessions`, every open session with its tab |
| `src/ui/search/history_text.rs` | Search Everywhere's History tab: full text over past agent conversations, one row per session, Enter resumes it |
| `crates/tty7-core/src/core/history_search.rs` | the History tab's scan: Claude, Qoder and Codex transcripts streamed newest first, what was said cached (`history_cache`); `session_mentions`, one session's issue and PR references (`#N`, `owner/repo#N`, `/pull/N` and `/issues/N` links), tool output included; only the shown repo's, a bare `#N` only when the session ran in a checkout of it, never a colour like `#333333` or `#0` |
| `crates/tty7-core/src/core/history_cache.rs` | the History tab's transcript cache: by path, size and mtime, least recently read dropped past the cap, deleted files forgotten |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/agent_history.rs` | `Found`, `claude_files`, `codex_files`, `codex_not_the_users`, `strip_injected`, `unix` made `pub(crate)` | `history_search` walks and filters the same files |
| `src/ui/app.rs` | `search_sessions` | its rows go through `open_first` |
| `src/ui/machine_mirror.rs` | `agent_sessions_of` | the sessions open in a mirrored workspace's tabs, for `open_agent_sessions` |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `SearchText` (Search Text in Files…) |
| `src/ui/search/sources.rs` | `Catalog` fields, `new`, `source`, `all` | `live` (`text::LiveTab`s); `all` leaves Text and History out; New Tab has no rows of its own; `Sessions::highlights` leaves open sessions to the Terminals rows |
| `src/ui/search/view.rs` | `perform_search`, `set_tab`, `render_empty`, `update_catalog` visibility | `Catalog::ask_live` for the tab showing; the too-short and remote hints |
| `src/ui/panel_search.rs` | module list | `pub(crate) mod model` for `split_relative` |
| `src/ui/app.rs` | `open_search` | `catalog.live = palette_live_tabs(..)` |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::SearchText` |
