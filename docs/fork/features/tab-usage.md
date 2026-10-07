# the sidebar shows each tab's pid, CPU and memory

`Fork-Feature: tab-usage`. The commit that carries this feature is `fork(tab-usage): …` on `main-niu`.

Each sidebar row ends with `pid 4821 · 12% · 3.0 GB`: the front pane's shell
pid, and the CPU% and resident memory of every process in every pane of the tab,
children included. One app-wide poll asks the daemon (or a remote pane's host)
for each pane's process tree every 3 seconds, the same `QueryProcs` Info →
Processes uses. A two-line row puts it at the end of the branch line; a one-line
row at the end of the title line. A sidebar too narrow to keep the title readable
drops it.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/mod.rs` | module list | `tab_usage` |
| `src/ui/tab_sidebar.rs` | the row loop: `TabUsage::want` before it; the usage label, its width taken from `label_avail`, and `usage_cell` on the branch or title line | the numbers and the room they take |
