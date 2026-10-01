# program notifications follow the per-pane rule

`Fork-Feature: program-notes`. The commit that carries this feature is `fork(program-notes): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/terminal/view/program_notes.rs` | which notices about a pane reach the desktop (command finish, agent, and what the program wrote over OSC 9/99/777), and showing the program's own, drained even after the shell exits, paced by `RemoteTerminal::pace_notes` with one "weren't shown" note for the rest |
| `src/terminal/remote/fork_pane.rs` | the fork's state on a pane's link, one `Signals` shared by `RemoteTerminal` and its readers, and `laid_out_grid` (the grid the pane last asked for, `None` until one was asked: what `background_grid` records from the shown tab and compares a never-shown pane against): the program's notes queue (capped) and per-pane `Budget` (a few per 10s), 2031 on/off; a reader's three calls `on_output` (queue notes, fold 2031, answer `?996n`), `on_replay`, `on_frame` (one 997 when a replay that left 2031 on ends: the first frame that is not `Size`/`Snapshot`); `take_osc_notes`, `pace_notes`, `color_scheme_updates` |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/terminal/view.rs` | `mod program_notes`; `poll_foreground` (first line, and `notify_allowed`); `NotifyMode` import | `program_notes::show` and `::allowed`: one rule for every notice about a pane, held back only from the focused pane of the key window |
| `docs/agents/status.mdx` | Notifications | program notifications and Claude Code's `/config` channel |
| `crates/tty7-core/src/core/term_modes.rs` | `TRACKED`, `COLOR_SCHEME_UPDATES`, `feed`'s `n` and OSC arms, `osc_end`, `take_color_scheme_queries` (+ tests) | track 2031 (a reattach restores it), count `?996n` queries, and drop 2031 at a shell prompt (OSC 133 `A`/`B`/`D`), so a dead program's 2031 never sends a 997 into the shell |
| `crates/tty7-core/src/daemon/pane.rs` | `INPUT_MODE_RESETS` | `?2031l`: a restored pane's new shell never asked for theme reports |
| `src/terminal/remote.rs` | `ReaderSignals` and `RemoteTerminal` (one `fork` field each, and in their literals), `mod fork_pane`, `spawn_reader`: `fork.reader()`, `flush_batch` (`on_output` in place of the direct `notify_desktop`), after each frame decodes (`on_frame`), `Snapshot` arm (`on_replay`) | program notes go to the view, not straight to the desktop; 2031 folded and reported (see `remote/fork_pane.rs`) |
