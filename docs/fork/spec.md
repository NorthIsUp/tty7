# Fork spec: clawmux parity on tty7

This fork (NorthIsUp/tty7) succeeds clawmux. clawmux's features land on tty7's
own server; tmux is not used. The fork stays easy to rebase on upstream `main`:
fork logic lives in fork-owned files, and upstream files carry only short hook
calls, each listed in `FORK.md`.

Already on the branch (PR #1): lazy restore, Continue All Agents /
`tty7-app --continue`, hook-free Claude resume via `--session-id`, `mise.toml`.

## F1 — New tab page

New Tab (⌘T, the `NewTab` action, the palette's New Tab) opens a centered modal
picker instead of a shell, when `new_tab_page` is on (default).

- **Kind row:** "Terminal", then every agent from `offered_agents()`, in that
  order. The initial selection is the most recently launched agent
  (`agent_launch::most_recent`), or Terminal when there is none. ⌃1..⌃9 pick by
  position, and Tab / ⇧Tab cycle. Clicking a chip selects it.
- **Directory list:** a query input plus a list, fuzzy-filtered with
  `ui::search::score::fuzzy_score`. Candidates, deduplicated by absolute path
  and ordered by score, then frecency, then source order:
  1. the active tab's cwd, which is the initial selection;
  2. every open tab's cwd in this window;
  3. `dir_frecency` entries whose directory still exists;
  4. the immediate child directories of each `dir_roots` entry, with `~`
     expanded and hidden directories skipped.

  Each row shows the path with `~` for home (`home::display_path`). ↑/↓ move,
  and the wheel scrolls.
- **Enter** opens a new tab in the selected directory: `new_tab_with_cwd` for
  Terminal, or the agent launch (with `--session-id` minting) for an agent. It
  bumps `dir_frecency[path]` (`count += 1`, `last_used = now`) and closes the
  picker. A query that is itself an existing directory path (`~` allowed) is
  offered as the first row.
- **Esc**, or a click outside the picker, closes it and opens nothing.
- `new_tab_page: false` restores upstream behaviour exactly.
- Remote workspaces are not affected: the page shows only for local
  workspaces, and remote ones keep upstream New Tab.

## F2 — Group colours

Each sidebar group header shows a small colour swatch before its name:

- The override is `group_colors[name]`, a `#rrggbb` hex string. An invalid
  value is ignored and logged once.
- Otherwise the colour is stable: a FNV-1a hash of the group name picks one of
  the theme's `ansi16` indices [1,2,3,4,5,6,9,10,11,12,13,14]. The same name
  gets the same colour in every window and across restarts, and nothing is
  stored.
- Ungrouped and SSH-host groups get a swatch too.

## F3 — Pane nice

On Unix, right after a pane's shell is spawned, the daemon calls
`setpriority(PRIO_PROCESS, shell_pid, Config::load().nice)` when `nice != 0`.
It clamps to 0..=19 and logs a failure without failing the spawn. Agents typed
into the shell inherit it. Native SSH panes have no local child and are
skipped. Nothing changes on Windows.

## F4 — Fork upkeep

- `FORK.md` at the repo root lists every fork-owned file and every hook point
  in an upstream file: file, function, one line on why.
- `mise run sync-upstream` fetches `upstream main` and rebases the current
  branch onto it. It stops on conflict with a message naming `FORK.md`.

## Out of scope

Status detection (tty7's hooks own it), iTerm2 integration (tty7 is the GUI),
and a plugin engine.
