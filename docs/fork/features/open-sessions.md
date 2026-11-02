# the Sessions tab leads with agent sessions already open in a tab

`Fork-Feature: open-sessions`. The commit that carries this feature is `fork(open-sessions): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/search/open_sessions.rs` | the Sessions tab's lead (⌘K): sessions open in a tab move to the front and go to that tab (`open_first`); `open_agent_sessions`, every open session with its tab |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/app.rs` | `search_sessions` | its rows go through `open_first` |
| `src/ui/machine_mirror.rs` | `agent_sessions_of` | the sessions open in a mirrored workspace's tabs, for `open_agent_sessions` |
