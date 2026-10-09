# a pane keeps its private modes across a daemon handoff

`Fork-Feature: handoff-modes`. The commit that carries this feature is `fork(handoff-modes): …` on `main-niu`. Upstream: l0ng-ai/tty7#1145.

A handoff rebuilt each pane with empty private modes, so a long-running TUI
lost what it switched on at startup once the ring dropped it: Claude Code
stopped following the theme (2031), and a reattach lost the alternate screen
and mouse, and kitty keyboard flags. The handoff record now carries the ring's
head fold, and the adopted pane derives its modes as that head plus the ring.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/daemon/handoff.rs` | `PaneRecord`, `stage`, `adopt`, a test | the record's `modes` field |
| `crates/tty7-core/src/daemon/pane.rs` | `Carried`, `Pane::carry`, `Pane::adopt`, `ReplayRing::adopted`, `ReplayRing::modes_from`, a test | carry `head_modes.restore_bytes()`, seed the ring's head and fold the pane's modes from it |
