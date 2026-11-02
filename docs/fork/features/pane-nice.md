# pane shells start at the configured nice

`Fork-Feature: pane-nice`. The commit that carries this feature is `fork(pane-nice): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/daemon/nice.rs` | `setpriority` on a pane's shell from `Config::nice` |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/daemon/pane.rs` | `spawn`, after `spawn_command` | `nice::apply(pid)` on the new shell |
