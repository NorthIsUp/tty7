# ⌘P takes a pane or tab address the way the CLI prints it

`Fork-Feature: pane-addresses`. The commit that carries this feature is `fork(pane-addresses): …` on `main-niu`.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/search/sources.rs` | `Terminals::search`, `parse_goto`, `goto_rows` | a typed `%N`/`@N` (or `pane N`/`tab N`) leads with a row that goes there, numbered as `tty7 pane ls` and `tty7 ls` number them |
| `src/ui/search/command.rs`, `src/ui/app.rs` | `CommandKind::GoToPane`; `run_command` | the pane row lands on its tab (waking it) and focuses the pane |
| `src/ui/i18n/{mod,en,ja,ru,zh}.rs` | before `CmdQuickConnect` | `CmdGoToPane`, `CmdGoToTabAddress` |
| `docs/window/search-everywhere.mdx` | Tabs table, Terminals | the pane and tab addresses |
