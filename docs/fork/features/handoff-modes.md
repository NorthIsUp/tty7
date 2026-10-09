# a pane keeps its private modes across a daemon handoff

`Fork-Feature: handoff-modes`. The commit that carries this feature is `fork(handoff-modes): …` on `main-niu`. Upstream: l0ng-ai/tty7#1145.

A handoff rebuilt each pane with empty private modes, so a long-running TUI
lost what it switched on at startup once the ring dropped it: Claude Code
stopped following the theme (2031), and a reattach lost the alternate screen
and mouse. The handoff record now carries the modes.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/daemon/handoff.rs` | `PaneRecord`, `stage`, `adopt`, a test | the record's `modes` field |
| `crates/tty7-core/src/daemon/pane.rs` | `Carried`, `Pane::carry`, `Pane::adopt` | write and fold back `restore_bytes()` |
