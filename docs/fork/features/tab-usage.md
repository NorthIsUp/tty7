# the sidebar shows each tab's pid, CPU and memory

`Fork-Feature: tab-usage`. The commit that carries this feature is `fork(tab-usage): …` on `main-niu`.

Settings → Tabs has a switch each for a sidebar row's pid, CPU and memory, all
off by default. The pid is the front pane's shell; CPU% and resident memory sum
every process in every pane of the tab, children included. *Annotate
high-resource tabs*, on by default, draws a shown CPU or memory figure in the
warning colour while it passes `HOT_CPU` or `HOT_RSS` (`src/ui/tab_usage.rs`).

The numbers are Info → Processes' figures, small, monospaced and padded to
fixed widths (` 1.0 GB`, `  12%`) so a moving value never shifts the row. One
app-wide poll asks the daemon, or a remote pane's host, for each pane's process
tree every 3 seconds, every pane at once (the same `QueryProcs` Info →
Processes uses); while the pid, CPU and memory switches are all off it stops. A two-line row puts the numbers at the end of the branch line, a
one-line row at the end of the title line, and a sidebar too narrow to keep the
title readable drops them.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/mod.rs` | module list | `tab_usage` |
| `src/ui/tab_sidebar.rs` | the row loop: `TabUsage::want` before it; `TabUsage::row` after `label_avail`, its element on the branch or title line, and the kept status slot | the numbers and the room they take |
| `src/ui/proc_usage.rs` | `format_cpu` | `pub(crate)`, so the row reuses the panel's CPU figure |
| `src/ui/settings/pages.rs` | the Tabs group, after the group header rows | `tab_usage_settings` |
| `crates/tty7-core/src/core/fork_config.rs` | `ForkConfig` | `tab_usage_pid`, `_cpu`, `_memory`, `_annotate` |
| `src/ui/i18n/{mod,en,ja,ru,zh}.rs` | after `NewTabPageDirsHeading` | `SettingsTabUsage*` titles and descriptions |
