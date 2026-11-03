# a Codex command run in another directory leaves the tab where it is

`Fork-Feature: codex-tool-cwd`. The commit that carries this feature is `fork(codex-tool-cwd): …` on `main-niu`.

Codex's `PostToolUse` and `PermissionRequest` hooks report the command's own
workdir as `cwd`. Taken as the session's cwd, one `git status` in another
checkout moved the tab into that repo's sidebar group and back with the next
command. Claude's tool hooks report the session's cwd, so they still move it.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/cli_agent.rs` | `AgentSessionState::apply_event` (+ test) | a Codex tool report's `cwd` is not the session's |
