# a new pane runs one command at its first prompt without making it the pane's shell

`Fork-Feature: run-once`. The commit that carries this feature is `fork(run-once): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/agent_resume.rs` | `spawn_job` / `landed`, a pending pane's spawn and landing: under `agent_resume_mode: program` (local, replacing a known pane, sh/bash/zsh/dash/ksh/fish) the resume rides the spawn as `RestoreFrom::run_once`, else it is typed at the first prompt, the plan read off the UI thread either way |
| `crates/tty7-core/src/daemon/run_once.rs` | a command a new pane runs once: `TTY7_RUN_ONCE` for an integrated zsh/bash/fish, else `-lic '<cmd>; exec <shell>'`, never the recorded shell (+ tests) |
| `crates/tty7-server/tests/run_once.rs` | zsh, bash and sh run it once, with integration and the user's `ZDOTDIR` (fish untested: not installed here) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/daemon/protocol.rs`, `server.rs`, `pane.rs` | `RestoreFrom::run_once`, `FEATURE_RUN_ONCE` (a client hands the command only to a daemon that lists it, else types it); the `Spawn` handler; `DaemonPane::spawn`, `build_spawn_config` (a `run_once` argument) (+ test) | `run_once::apply`: the pane records its plain shell, never the command |
| `crates/tty7-core/src/daemon/shell_integration.rs` | the zsh script's `__tty7_run_once_line` (a `line-init` widget), the bash script's `__tty7_run_once` (at the top of `__tty7_precmd`), the fish script's `__tty7_run_once` (in `__tty7_precmd`) | run `TTY7_RUN_ONCE` once at the first prompt (zsh as an accepted line, after `ZDOTDIR` is restored), out of the environment first |
| `crates/tty7-core/src/core/shell_quote.rs` | `runs_once` | the shells a command can be handed to run once |
