# the terminal answers DA1, DA2 and XTVERSION

`Fork-Feature: terminal-id-replies`. The commit that carries this feature is `fork(terminal-id-replies): …` on `main-niu`.

Carries upstream [#1343](https://github.com/l0ng-ai/tty7/pull/1343) until it merges; sync drops this commit then.

## Hooks in upstream files

| file | why |
|---|---|
| `crates/tty7-cli/src/backend/real.rs` | upstream #1343 |
| `crates/tty7-core/src/core/term_modes.rs` | upstream #1343 |
| `crates/tty7-core/src/daemon/pane.rs` | upstream #1343 |
| `src/terminal/remote.rs` | upstream #1343 |
