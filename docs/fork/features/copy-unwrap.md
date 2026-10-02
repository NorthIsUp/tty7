# Copy rejoins lines Claude Code wrapped

`Fork-Feature: copy-unwrap`. The commit that carries this feature is `fork(copy-unwrap): …` on `main-niu`.

Claude Code (Ink) wraps its own output: at the pane's width it writes CRLF and
the block's indent, so the grid never sets WRAPLINE and a copy keeps every
break. Soft-wrapped rows already join (alacritty's `selection_to_string`
honours WRAPLINE). This applies to Claude panes only: with
`copy_join_wrapped` on (off by default) and Claude Code the pane's foreground
agent, copy joins row N to N+1, dropping N+1's indent, when all of the rules
below hold. Every other pane copies raw, since `ls` columns, test separators,
an editor's own wraps, `git log --oneline` and JSONL all look like Claude's
wraps. ⌘⌥C (hard-coded beside ⌘C) and Copy Raw always copy as shown.

- N is full: its last cell + 1 + N+1's first word ≥ the pane's columns (Ink
  breaks one cell early when the word would land on the edge), and that word
  fits on a row at all.
- N+1's indent is between the run's first row's indent and where its text
  starts after a marker (`⏺ `, `- `, `1. `, `⚠️ `).
- N+1 starts with ASCII or a letter, not a symbol (`⏵⏵`, `◯`), and is not a
  list item; N doesn't end in `…` (truncated, not wrapped).
- The join is one space, or nothing when Ink cut a word: the row is one word
  filling the row to the edge (Ink's hard breaks fill to exactly the pane's
  width in the captures; its word wraps stop short), or the characters on both
  sides of the boundary are wide (CJK).
- Neither row holds box drawing or block elements, or starts with `|`, `$ `,
  `> `, `❯ `, `` ``` `` or `⎿`; not inside a fence; not inside a `⎿` tool
  result, whose rows are char-wrapped.

Measured on 16 captures of Claude panes (15 at 111 columns, one at 172): 236
joins, all with a space, no false joins found on review. Missed: one system
message Claude wraps narrower than the pane (4 copies of it); tool calls and
tool output stay raw by design. Claude prints code blocks unfenced at the
block's indent; a code line Ink wrapped joins back into the one command it was
(`fixtures/copy_unwrap_code.txt`, its repo names replaced at equal width), and
separate code lines are short, so they never look full. Block selections copy
raw. Hyphenated breaks aren't treated specially: Ink wraps words at spaces and
cuts only a word too long for a row, which the one-word rule covers.

Known false joins left in:

- The gate is the pane's foreground agent, not the rows: shell output still
  in the scrollback above where `claude` started joins like Claude's when a
  selection reaches back into it.
- Two separate code lines at the same indent join when the first nearly fills
  the row (the next line's first word would not fit after it). Claude's code
  lines rarely run that long; none did in the captures.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/terminal/view/copy_unwrap.rs` | the join rule, `TerminalView::copy_raw`, the Settings row (`Tty7App::copy_unwrap_settings`), tests |
| `src/terminal/view/fixtures/copy_unwrap_*.txt` | `tty7 capture --plain` excerpts the tests run on |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/terminal/view.rs` | module list; `copy_selection` → `copy_selection_as(raw)`; key handler before the ⌘ shortcuts | `copy_unwrap::selection_text`; ⌘⌥C → `copy_raw` |
| `crates/tty7-core/src/core/fork_config.rs` | `ForkConfig::copy_join_wrapped` | the setting, `false` |
| `src/ui/settings/pages.rs` | Selection & Clipboard group | chain `copy_unwrap_settings` |
| `src/ui/settings.rs`, `src/ui/app.rs` | `settings_search_entries`, `config_key`, `description`, `modified`; `reset_settings_value` | the row in Settings search, its modified dot and reset |
| `src/ui/search/command.rs`, `src/ui/app.rs` | `CommandKind::CopyRaw`, `id`, `key_spec`, terminal items; `run_command` | Copy Raw in the palette |
| `src/ui/i18n/{mod,en,ja,zh}.rs` | fork block | `CmdCopyRaw`, `SettingsCopyJoinWrapped`(`Desc`), `SettingsSearchCopyJoinWrappedKeywords` |
| `docs/reference/configuration.mdx` | config table | `copy_join_wrapped` |
