# a new or deleted file's diff draws unified

`Fork-Feature: diff-one-sided`. The commit that carries this feature is `fork(diff-one-sided): …` on `main-niu`.

A new or deleted file has one side only, so in the split view half the card was
empty. Such a file now draws unified whatever the view; the others still split.
A drag takes its mode from the row it starts on, so selecting and copying in
one of these files works in either view.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/diff_list.rs` | `file_mode`, called at the top of `file_rows` | added or deleted → unified |
| `src/ui/diff_overlay.rs` | `sync_diff_rows`, the stale-selection check | compare against the file's own mode, not the view's |
| `src/ui/diff_overlay.rs` | `diff_row_drag`; `Drag` loses `mode` | the selection's mode comes from the row it started on |
