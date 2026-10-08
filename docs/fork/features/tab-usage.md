# the sidebar shows each tab's pid, CPU and memory

`Fork-Feature: tab-usage`. The commit that carries this feature is `fork(tab-usage): …` on `main-niu`.

Settings → Tabs has a switch each for a sidebar row's pid, CPU and memory, all
off by default. The pid is the front pane's shell; CPU% and resident memory sum
every process in every pane of the tab, children included. *Annotate
high-resource tabs*, on by default, shows a tab's CPU or memory in the warning
colour while it passes 80% or 4 GB, even with its switch off.

The numbers are small, monospaced and padded to fixed widths (`  1.0 GB`,
` 12.0%`) so a moving value never shifts the row. One app-wide poll asks the
daemon, or a remote pane's host, for each pane's process tree every 3 seconds
(the same `QueryProcs` Info → Processes uses), and asks for nothing while every
switch is off. A two-line row puts the numbers at the end of the branch line, a
one-line row at the end of the title line, and a sidebar too narrow to keep the
title readable drops them.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/mod.rs` | module list | `tab_usage` |
| `src/ui/tab_sidebar.rs` | the row loop: `TabUsage::want` before it; the cells, their width taken from `label_avail`, and `usage_cell` on the branch or title line | the numbers and the room they take |
| `src/ui/settings/pages.rs` | the Tabs group, after the group header rows | `tab_usage_settings` |
| `crates/tty7-core/src/core/fork_config.rs` | `ForkConfig` | `tab_usage_pid`, `_cpu`, `_memory`, `_annotate` |
| `src/ui/i18n/{mod,en,ja,ru,zh}.rs` | after `NewTabPageDirsHeading` | `SettingsTabUsage*` titles and descriptions |
