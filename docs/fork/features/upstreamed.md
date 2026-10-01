# Merged upstream on 2026-10-01; drops out at the next sync

Rows for fork work upstream merged as #1059–#1064 (process CPU and memory,
DEC 2031, OSC 99, Move to Group, the sidebar scroll fix, `.itermcolors`
integers). The next `mise run sync-upstream` drops the fork's copies; then
this page goes too.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/daemon/procstat.rs` | per-process RSS, CPU time and start stamp for Info → Processes; `compact_bytes` |
| `src/ui/proc_usage.rs` | CPU% from two samples, the Processes row's CPU / memory / pid cells and its Total line |
| `src/terminal/color_scheme.rs` | DEC mode 2031: `CSI ? 997 ; 1\|2 n` to a pane whose program switched it on when the theme's background changes, and the `CSI ? 996 n` answer (Claude Code's `theme: auto` re-reads OSC 11 only on a 997) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/osc.rs` | `Notifications` (+ tests) | OSC 99 (kitty) notifications, chunked by `i=`, beside OSC 9/777; offered upstream as `upstream/osc-notifications` |
| `crates/tty7-core/src/daemon/pane.rs` | `OscSniffer` (+ test) | read OSC 99 too, so a hookless agent's kitty notification marks it Waiting |
| `src/ui/tab_strip.rs`, `src/ui/tab_sidebar.rs`, `src/ui/i18n/{mod,en,zh,ja}.rs` | `tab_context_menu` (Move to Group submenu), `move_targets`, `move_tab_to`, `pin_auto_group_with` (+ tests); `SidebarRemoveFromGroup` | Move to Group lists every sidebar group, auto ones too (picking one pins it), and Remove from Group, its check on the right so the labels line up with the tab menu; offered upstream as `upstream/move-to-group-submenu` |
| `src/terminal/remote.rs` | `OscNotifyScanner` (+ tests) | OSC 99 and a note's multi-part state (`osc::Notifications`); `feed` extends any queue |
| `crates/tty7-core/src/core/config.rs` | `NotifyMode::allows` (+ test) | the notice policy `view/program_notes.rs` applies; identical to upstream/osc-notifications |
| `crates/tty7-core/Cargo.toml` | `windows-sys` features | `Win32_System_ProcessStatus` for `procstat`'s working set |
| `crates/tty7-core/src/daemon/protocol.rs` | `ProcEntry` | `rss`, `cpu_ns`, `started` (serde default), `Default` derive |
| `crates/tty7-core/src/daemon/procinfo.rs` | `snapshot`, `walk` | `procstat::fill` on the trimmed list; `..Default::default()` in `ProcEntry` literals |
| `crates/tty7-core/src/host/server.rs`, `crates/tty7-cli/src/commands.rs` | test `ProcEntry` literals | `..Default::default()` |
| `crates/tty7-cli/src/output.rs` | `procs_tables` (+ test) | RSS column appended |
| `src/ui/right_panel.rs` | `RightPanelState::cpu`, `procs_section`, `spawn_procs_query` | sample CPU on each poll; `proc_usage` cells per row and the Total line |
| `src/ui/mod.rs`, `src/ui/i18n/{mod,en,zh,ja}.rs` | module list, `PanelProcessesTotal` | `proc_usage`; "Total" |
| `src/ui/presets.rs` | `load_iterm_theme` colour component (+ test) | read `<integer>` components, which iTerm2 and plistlib write for exact 0 and 1; offered upstream as `upstream/itermcolors-integer` |
| `src/ui/app.rs`, `src/ui/tab_sidebar.rs` | `Tty7App::sidebar_reveal` (by `TabId`), `activate`; `tab_sidebar` row canvas and the undrawn-reveal clear, `reveal_shift` (+ test) | selecting a tab scrolls its row in only when it is out of view, at the nearest edge; identical to upstream/sidebar-scroll-nearest |
| `src/terminal/view.rs` | `with_terminal` (made `pub(super)` for the test) | `color_scheme::watch` |
| `src/terminal/mod.rs` | module list | `mod color_scheme` |
