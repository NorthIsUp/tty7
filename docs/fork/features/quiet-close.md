# an untouched new tab closes without asking

`Fork-Feature: quiet-close`. The commit that carries this feature is `fork(quiet-close): …` on `main-niu`.

With `quiet_close` on (the default), closing a tab or pane skips the
`confirm_close` question when every pane in it was opened this run and has had
no keystroke, paste or drop. Lines tty7 types itself (an agent launch, a
worktree's setup, a resume) don't count. The per-host SSH warning still asks.

A pane is marked untouched only where the user opens one: a new tab, a split,
an SSH tab, a forked agent, a worktree tab, the first tab of a sessionless
launch. Restore, wake, respawn and a reattached view never mark, so they keep
today's question. A still-dialling pane carries the mark on its
`PendingSpawn` and drops it if the landing reattached.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/terminal/view.rs` | `TerminalView::untouched`, set false in `on_key_down`, `commit_text`, `paste`, `paste_from_clipboard`, `drop_files`, `drop_remote_path` | the per-pane mark |
| `src/ui/app.rs` | `mark_untouched` at the new-tab and split sites, `land_pane` | where a pane counts as fresh |
| `src/ui/app.rs` | `leaf_close_reason_as`, `tab_close_reason` | an untouched pane closes as under `Never` |
| `src/ui/app.rs` | `deliver_agent_prompt` | a prompt sent to an agent counts as input |
| `src/ui/pending_pane.rs` | `PendingSpawn::untouched` | the mark across a pending spawn |
| `src/ui/settings.rs`, `src/ui/settings/pages.rs`, `src/ui/i18n/*.rs` | the General → Tabs row | the setting |
| `crates/tty7-core/src/core/fork_config.rs` | `ForkConfig::quiet_close` | the key |
