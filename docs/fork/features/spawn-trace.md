# each new tab logs its start as one timeline

`Fork-Feature: spawn-trace`. The commit that carries this feature is `fork(spawn-trace): …` on `main-niu`.

Every stage of a new tab's start logs one `spawn-trace` info line to
`tty7.log`, once. The app's lines carry `attempt=N` (minted when the New Tab
page opens, or in `new_tab_slot` when it is off) and add `pane=N` once the
daemon's `Spawned` reply names the pane; the daemon's carry `pane=N`, so the two
join on it. `ms` counts from that process's first stage for the tab: the app's
from the New Tab page opening, the daemon's from `DaemonPane::spawn` starting.

```
… INFO  tty7 … spawn-trace attempt=4 stage=new_tab_page_opened ms=3.2 target=tab
… INFO  tty7 … spawn-trace attempt=4 stage=picked ms=1840.7 kind=agent:claude dir=row
… INFO  tty7 … spawn-trace attempt=4 stage=spawn_sent ms=1852.0
… INFO  server … spawn-trace pane=12 stage=child_started ms=10.5
… INFO  tty7 … spawn-trace attempt=4 pane=12 stage=spawned ms=1866.3
… INFO  server … spawn-trace pane=12 stage=first_output ms=54.6
… INFO  tty7 … spawn-trace attempt=4 pane=12 stage=first_output ms=1911.9
… INFO  tty7 … spawn-trace attempt=4 pane=12 stage=first_paint ms=1915.4
… INFO  server … spawn-trace pane=12 stage=prompt ms=55.5
… INFO  server … spawn-trace pane=12 stage=command ms=1210.0
```

App stages: `new_tab_page_opened` (`target=tab|split`) or `tab_requested`,
`picked` (`kind=terminal|agent:<slug>`, `dir=typed|row`; never the path),
`spawn_sent` (just before the blocking local spawn call), `spawned`,
`first_output` (the view's first `Wakeup` once the link has carried pane
bytes; a new pane's empty attach replay does not count), `first_paint` (the first frame built
after it; a background tab logs it only once shown). Daemon stages
(the existing `pane N spawn requested` line marks the request): `child_started`,
`first_output`, `prompt` (first 133;A/B),
`command` (first 133;C, never its text). The daemon's reader drops its trace
after `command`; a shell without integration keeps it, at one branch per read.
A restored pane is not traced: the app has no attempt to join it to.
Only a local spawn is traced in the app; a routed one drops its attempt.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/core/spawn_trace.rs` | `Trace`, the line format, the daemon's parked traces, the UI thread's hand-off |
| `crates/tty7-server/tests/spawn_trace.rs` | a real zsh pane's daemon stages in its `tty7.log` |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/mod.rs` | module list | `spawn_trace` |
| `crates/tty7-core/src/daemon/pane.rs` | `DaemonPane::spawn`: top; after the `child started` log; after `take_writer` | `Trace::pane` unless restoring; `child_started`; `park` |
| `crates/tty7-core/src/daemon/pane.rs` | `spawn_reader`, thread start; loop, after `suppress_relayed_prompt_marks` (with `saw_prompt_mark`/`ran` hoisted there), before the foreground probes | `unpark`; `Trace::read`, dropped when done |
| `src/ui/app.rs` | `new_tab_slot`, after `guard_local_spawn` | `request_tab` |
| `src/ui/app.rs` | `new_terminal`, local branch | `spawn_sent`, `spawned`, the trace onto the view |
| `src/terminal/view.rs` | `TerminalView` field + literal; `handle_event` `Wakeup` | `spawn_trace`; `first_output`, gated on `saw_output` |
| `src/terminal/remote.rs` | `ReaderSignals`/`RemoteTerminal` field, both constructions; before `match msg`; `saw_output()` | non-empty pane bytes seen |
| `src/terminal/element.rs` | `paint`, the `grid_snap` update | `painted` |
| `src/ui/new_tab_page.rs` (palette-tabs) | `NewTabPage.trace`, `open`, `open_page` | `new_tab_page_opened`, `picked`, `handing` round the spawn |
| `docs/reference/troubleshooting.mdx` | its own "A new tab is slow to start" section, before "Panes came back empty" | the grep |
