# a pane no window shows still answers DA1

`Fork-Feature: detached-replies`. The commit that carries this feature is `fork(detached-replies): …` on `main-niu`.

The client's emulator answers a pane's queries, so a pane with no client
attached answers nothing, and the replay on attach drops replies. Claude Code
ends its OSC 11 theme query with DA1 and waits for both; started in such a pane
(an agent woken at launch), it hangs on that query and skips every later theme
check, so it keeps the dark theme whatever tty7 reports. The server now answers
DA1 itself while no client is attached; the batch ends, and the colour report a
client sends on attach makes Claude Code ask again.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/term_modes.rs` | `TerminalModes::feed`, `take_device_attribute_queries` | count `CSI c` / `CSI 0 c` |
| `crates/tty7-core/src/daemon/pane.rs` | the reader loop after `fan_out_output`; `answer_detached_queries`, `DA1_REPLY` | answer them while `subscriber` is `None` |
