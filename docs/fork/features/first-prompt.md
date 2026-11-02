# a typed launch waits for the shell's first prompt

`Fork-Feature: first-prompt`. The commit that carries this feature is `fork(first-prompt): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/first_prompt.rs` | typing a quick launch's line, or a typed resume, at a new shell's first prompt: `type_at_first_prompt`, and `prompt_patience` (30s for a shell with integration, 3s without) (+ tests) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/agent_launch.rs` | `run_when_ready` (Ready arm; `PROMPT_WAIT`/`PROMPT_POLL` removed) | one call to `first_prompt::type_at_first_prompt`, the wait every typed launch goes through |
| `crates/tty7-core/src/daemon/pane.rs`, `shell_integration.rs` | `integrates` (+ test) | whether a spawn gets integration, from the daemon's own shell choice, so the wait knows a prompt is coming |
