# Fork TODO

## 2026-09-28 — pane nice rechecked on a timer

- **Where:** crates/tty7-core/src/daemon/nice.rs `apply`
- **What:** after `setpriority`, the value is checked again at 1s and 5s and reapplied if something reset it.
- **Why:** on macOS, shells spawned in the daemon's first moments (restored panes) were read back at 0; later spawns kept the value. The cause is outside tty7, probably a task policy applied after the daemon disclaims responsibility, and it was not pinned down.
- **Fix:** find what resets it (run `fs_usage`/dtrace on the daemon at launch) and apply at the right moment, or set the priority in the child before exec if portable-pty ever grows a pre_exec hook.

## 2026-09-28 — new tab page counts a use before the spawn lands

- **Where:** src/ui/new_tab_page.rs `commit_new_tab_page`
- **What:** `dir_frecency` is bumped even when the tab then fails to spawn.
- **Why:** the spawn reports failure asynchronously, and a stray count is harmless.
- **Fix:** bump it from `land_pane` for a pane opened by the page.

## 2026-09-28 — fork CI is off

- **Where:** github.com/NorthIsUp/tty7/actions
- **What:** GitHub does not run workflows on a fork until someone clicks "enable workflows" in the UI; no API does it.
- **Why:** it needs a human click.
- **Fix:** click it; `ci.yml` then runs on PRs. Leave `nightly.yml` and `release.yml` disabled, since they publish upstream's artifacts.

## 2026-09-28 — Claude resume falls back with `||`

- **Where:** src/ui/app.rs `agent_resume_command`, crates/tty7-core/src/core/cli_agent.rs `start_command`
- **What:** a Claude resume is typed as `claude --resume <id> || claude --session-id <id>`, so a conversation that was never saved starts fresh instead of stopping at "No conversation found".
- **Why:** telling "never saved" from "saved" needs a look at `~/.claude/projects`, and the GUI may not touch the disk on the UI thread. The shell's exit status is free.
- **Fix:** resolve it off-thread through `agent_history` before typing, and pick one command. The `||` also fires when a resumed session later exits non-zero; the fresh start then fails fast on the id already in use.
