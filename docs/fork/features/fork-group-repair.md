# one-shot repair of tabs an older fork build filed in the wrong folder group

`Fork-Feature: fork-group-repair`. The commit that carries this feature is `fork(fork-group-repair): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/core/group_migrate.rs` | `run_once`: a one-shot repair of tabs an older build misfiled in folder groups, marked done by `group-membership-repaired` beside `machine.json` once the tree is written (a fresh install with its first save, a failed load never): the live rule on each tab's last cwd and repo, symlinks resolved and unresolved paths compared as spelled too; with no cwd, a tab whose repo holds its folder or sits under another pinned one is left alone; a tab whose paths do not resolve is skipped (+ tests) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/machine.rs` | `MachineStore::open` calls `group_migrate::run_once` (an empty tree re-parsed to tell it from a failed load) and flushes; `mark_repaired`, and `persist` writes the marker after the tree | the one-shot repair runs on the tree every window reads, and is recorded only once that tree is on disk |
