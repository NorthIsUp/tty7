# Search Everywhere's History tab searches past agent sessions

`Fork-Feature: search-history`. The commit that carries this feature is `fork(search-history): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/core/fork_host.rs` | `ForkHost` (history search, session mentions) behind `Host::fork`: the local host's answers, and `NoFork`'s nothing-found for every other host |
| `src/ui/search/history_text.rs` | Search Everywhere's History tab: full text over past agent conversations, one row per session, Enter resumes it |
| `crates/tty7-core/src/core/history_search.rs` | the History tab's scan: Claude, Qoder and Codex transcripts streamed newest first, what was said cached (`history_cache`); `session_mentions`, one session's issue and PR references (`#N`, `owner/repo#N`, `/pull/N` and `/issues/N` links), tool output included; only the shown repo's, a bare `#N` only when the session ran in a checkout of it, never a colour like `#333333` or `#0` |
| `crates/tty7-core/src/core/history_cache.rs` | the History tab's transcript cache: by path, size and mtime, least recently read dropped past the cap, deleted files forgotten |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/host/mod.rs`, `host/local.rs` | `Host::fork` (default `NoFork`; the local host is its own `ForkHost`) | the fork's host calls (`fork_host`), so the UI never reads the history files |
| `crates/tty7-core/src/core/agent_history.rs` | `Found`, `claude_files`, `codex_files`, `codex_not_the_users`, `strip_injected`, `unix` made `pub(crate)` | `history_search` walks and filters the same files |
