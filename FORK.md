# The NorthIsUp tty7 fork

This is [l0ng-ai/tty7](https://github.com/l0ng-ai/tty7) plus agent resume after
a reboot (Continue All Agents), a new tab page, a Text (find in files) tab and a History (agent conversations) tab in
Search Everywhere, sidebar group colours and a niceness for pane shells. It stays rebasable by one rule: fork logic lives in
fork-owned files, and an upstream file gets only a short hook call into them,
listed below. The fork's work lives on `main-niu` (the default branch); `main`
mirrors upstream and never gets fork commits. `mise run sync-upstream` syncs
`main` from upstream and rebases the current branch onto `upstream/main`; when
it stops on a conflict, the hook table says what each fork hunk in that file is
for, so keep upstream's side, re-add the hook, and `git rebase --continue`. A
fork commit upstream has since merged is dropped from the rebase rather than
resolved. Rebasing `main-niu` rewrites it, so it goes back with
`git push --force-with-lease`, and open fork branches rebase onto it.

## The app

`mise run install-app` builds the fast profile into
`~/Applications/tty7-niu-dev.app` (`com.northisup.tty7-niu-dev`), signed with
the keychain's Developer ID Application identity (or `TTY7_SIGN_ID`). The
designated requirement names the bundle id and team, not the build, so macOS
privacy grants survive every reinstall. The running app watches the bundle's
`local-build-id` and offers a restart when a new build lands; it never checks
GitHub. `mise run launch` and `reload` run that bundle.

A merge to `main-niu` that bumps the workspace `version` in `Cargo.toml` gets tagged
`v<version>` by `tag-on-bump.yml`, which starts `release.yml` on that tag.
CI (`release.yml`, on a `v*` tag) ships `tty7-niu.app` (`com.northisup.tty7-niu`),
notarized, and publishes the release here; its updater reads this repo's
releases. The signing secrets come from `! mise run set-release-secrets`.

## Fork-owned files

| file | what it holds |
|---|---|
| `FORK.md` | this page |
| `mise.toml`, `mise-tasks/` | tool pin; build, test, release and `sync-upstream`; `build-fast`, `install-app` (the fast build as `~/Applications/tty7-niu-dev.app`, signed), `launch` (that bundle, clean env) and `reload` (restarts the server in place when its code changed, so panes survive, then replaces the window); `set-release-secrets` (human-run, release.yml's signing secrets) |
| `src/core/fork_update.rs` | the update feed's repo (`update_repo!`, NorthIsUp/tty7); a local install checks no feed and prompts to restart when `install-app` lands a new build |
| `docs/fork/**` | spec, master plan and task plans for the fork |
| `docs/window/new-tab-page.mdx` | user docs for the new tab page |
| `src/ui/agent_resume.rs` | Continue All Agents (every sleeping agent tab), `--continue` and the launch/restart wakes (only tabs restore found dead, never hibernated ones: `RestoredDead`, `Wake`), which dead tabs restore asleep; the Resume agents on restart row, the restart wake and the quit/restart dialog copy; `wake_tab_with` (a wake whose resumes carry a prompt, via the scoped `WakePrompt`); `Resume`/`AtPrompt`, the one command a restored agent pane types; `prompt_patience` |
| `src/ui/new_tab_page.rs` | the new tab page picker (agent or terminal, and a directory), drawn as Search Everywhere's New Tab tab |
| `src/ui/background_tab.rs` | ⇧ opens a tab in the background: `in_background`, `seat_new_tab`, which palette rows take it, where `active` lands; `bind_shift_enter`, the palette list's ⇧Enter |
| `src/ui/search/text.rs` | Search Everywhere's Text tab: find in files over `Host::search_content`, debounced, never on All; `LiveTab`, the per-palette ask, debounce and latest-answer check History shares |
| `src/ui/search/agents.rs` | Search Everywhere's Agents tab (⌘K): the Terminals tab's open tabs, then the Sessions tab's rows not open in any pane |
| `src/ui/search/history_text.rs` | Search Everywhere's History tab: full text over past agent conversations, one row per session, Enter resumes it |
| `crates/tty7-core/src/core/history_search.rs` | the History tab's scan: Claude, Qoder and Codex transcripts streamed newest first, what was said cached (`history_cache`); `session_mentions`, one session's issue and PR references, tool output included; a bare `#N` only when the session ran in a checkout of the shown repo |
| `.github/workflows/tag-on-bump.yml` | on a `main-niu` push that bumps the workspace version: tag `v<version>` and dispatch `release.yml` on it |
| `src/ui/group_color.rs` | a group's colour (override, else a golden-angle hue by sidebar place; Ungrouped grey) and its swatch |
| `src/ui/group_header.rs` | a group header's outline and fill (header or whole group), its chevron, the fold slide, the repo default branch it names, and their Settings rows |
| `src/ui/github_session.rs` | the GitHub tab's Session list, its default: every issue and pull request the focused pane's agent session mentions, latest first, rows from the lists, details and a capped one-at-a-time lookup; `remote_pick`, the fork before upstream |
| `crates/tty7-core/src/daemon/nice.rs` | `setpriority` on a pane's shell from `Config::nice` |
| `crates/tty7-core/src/daemon/procstat.rs` | per-process RSS, CPU time and start stamp for Info → Processes; `compact_bytes` |
| `src/ui/proc_usage.rs` | CPU% from two samples, the Processes row's CPU / memory / pid cells and its Total line |
| `src/terminal/element/osc8_underline.rs` | an OSC 8 link's resting faint dotted underline (iTerm2's), solid under the pointer; an SGR underline keeps its own |
| `crates/tty7-core/src/core/claude_background.rs` | Claude sessions running in the background: session ↔ job id from `sessions/<pid>.json`; `resume_plan` (attach, resume, or start fresh when nothing was saved); `adopt_argv_session`, the session id an agent's argv names, with the miss cached per argv |
| `crates/tty7-core/src/core/fork_host.rs` | `ForkHost` (history search, session mentions, resume plan) behind `Host::fork`: the local host's answers, and `NoFork`'s nothing-found for every other host |
| `crates/tty7-core/src/core/history_cache.rs` | the History tab's transcript cache: by path, size and mtime, least recently read dropped past the cap, deleted files forgotten |
| `src/terminal/view/program_notes.rs` | which notices about a pane reach the desktop (command finish, agent, and what the program wrote over OSC 9/99/777), and showing the program's own, drained even after the shell exits, paced by `RemoteTerminal::pace_notes` with one "weren't shown" note for the rest |
| `src/terminal/remote/fork_pane.rs` | the fork's state on a pane's link, one `Signals` shared by `RemoteTerminal` and its readers: the program's notes queue (capped) and per-pane `Budget` (a few per 10s), 2031 on/off; a reader's three calls `on_output` (queue notes, fold 2031, answer `?996n`), `on_replay`, `on_frame` (one 997 when a replay that left 2031 on ends: the first frame that is not `Size`/`Snapshot`); `take_osc_notes`, `pace_notes`, `color_scheme_updates` |
| `crates/tty7-core/src/core/fork_config.rs` | `ForkConfig`: the fork's settings (resume, new tab page, group decoration, `nice`, GitHub panel, global hotkey), flattened into `Config`; `GroupColorSource`, `GroupBackgroundScope`. The retired `github_panel_default_list` / `github_panel_session_filter` keys still load (ignored) |
| `src/ui/hotkey_window/` | the global hotkey (`global_hotkey`, ⌥Space), macOS only: `mod.rs` the platform-free rules (toggle, the lift stack, restore, the switcher row's item and badge, the Workspaces menu label); `carbon.rs` the chord table and `RegisterEventHotKey`; `appkit.rs` the one dedicated hotkey window (its workspace saved in `hotkey-window`) shown / focused / ordered out with a fade, full screen (floating level under Force Quit, the Dock and menu bar moved aside by presentation options while it is the key window, and only what it set put back), windows activated over it lifted above it, hide on focus loss; `settings.rs` its Settings rows |
| `docs/window/hotkey-window.mdx` | user docs for the hotkey window |
| `src/terminal/color_scheme.rs` | DEC mode 2031: `CSI ? 997 ; 1\|2 n` to a pane whose program switched it on when the theme's background changes, and the `CSI ? 996 n` answer (Claude Code's `theme: auto` re-reads OSC 11 only on a 997) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/osc.rs` | `Notifications` (+ tests) | OSC 99 (kitty) notifications, chunked by `i=`, beside OSC 9/777; offered upstream as `upstream/osc-notifications` |
| `crates/tty7-core/src/daemon/pane.rs` | `OscSniffer` (+ test) | read OSC 99 too, so a hookless agent's kitty notification marks it Waiting |
| `src/ui/tab_strip.rs`, `src/ui/tab_sidebar.rs`, `src/ui/i18n/{mod,en,zh,ja}.rs` | `tab_context_menu` (Move to Group submenu), `move_targets`, `move_tab_to`, `pin_auto_group_with` (+ tests); `SidebarRemoveFromGroup` | Move to Group lists every sidebar group, auto ones too (picking one pins it), and Remove from Group; offered upstream as `upstream/move-to-group-submenu` |
| `src/ui/tab_sidebar.rs` | header pin mark (clickable on label groups too, `debug_selector`), tests `clicking_a_{folder,label}_groups_pin_unpins_it` | a click on any pinned group's pin unpins it; the label-group half offered upstream as `upstream/label-group-unpin` |
| `src/terminal/remote.rs` | `OscNotifyScanner` (+ tests) | OSC 99 and a note's multi-part state (`osc::Notifications`); `feed` extends any queue |
| `src/terminal/view.rs` | `mod program_notes`; `poll_foreground` (first line, and `notify_allowed`); `NotifyMode` import | `program_notes::show` and `::allowed`: one rule for every notice about a pane, held back only from the focused pane of the key window |
| `docs/agents/status.mdx` | Notifications | program notifications and Claude Code's `/config` channel |
| `src/terminal/element.rs` | `mod osc8_underline`, `RenderCell::osc8_dots`, end of `snapshot_cell`, `flag_hovered_link`; test `test_colors` made `pub(super)` | `osc8_underline::mark` / `unmark` |
| `Cargo.toml` | `[profile.fast]` | the day-to-day build: deps at opt 3, the app crate at opt 1, no LTO |
| `src/core/update.rs` | `REPO`, `RELEASES_URL`, `NIGHTLY_RELEASE_URL` | `update_repo!()`, so checks and links read the fork's releases |
| `src/core/update.rs` | `spawn_check`, `spawn_check_inner` | `fork_update::watch`; a local install skips the GitHub check |
| `src/core/mod.rs` | module list | `pub mod fork_update` |
| `src/bin/tty7-updater.rs` | `install_inner`, `extract_archive` → `unpacked_app` (+ test) | find the unpacked `.app` rather than name `tty7.app`, since the fork's is `tty7-niu.app` |
| `.github/scripts/bundle-macos.sh` | top, Info.plist, signing, notarization, after the sweep | `TTY7_APP_NAME`, `TTY7_BUNDLE_ID`, `TTY7_BIN_DIR`, `TTY7_DIST`, `TTY7_LOCAL_BUILD_ID`, `TTY7_BUNDLE_ONLY`; sign with a keychain identity when no cert is imported (no timestamp); notarize with an ASC API key (`ASC_KEY_P8`, `ASC_KEY_ID`, `ASC_ISSUER_ID`) |
| `.github/workflows/ci.yml` | `on.push.branches`, the three `save-if`s; `build` matrix, `server-musl` `if` | `main-niu`, not `main`; macOS only, Windows, Linux and musl commented out |
| `.github/workflows/release.yml` | `Bundle macOS DMG` env; `draft-release` last step | build `tty7-niu.app` (`com.northisup.tty7-niu`) with the ASC notarization key; publish the draft on NorthIsUp/tty7 |
| `crates/tty7-core/src/core/config.rs` | `Config::fork` (`#[serde(flatten)]`), `Default` | every fork setting lives in `fork_config::ForkConfig`, at the top level of `config.json` as before |
| `crates/tty7-core/src/core/config.rs` | `NotifyMode::allows` (+ test) | the notice policy `view/program_notes.rs` applies; identical to upstream/osc-notifications |
| `crates/tty7-core/src/core/cli_agent.rs` | `CLIAgent::resume_takes_prompt`, `CLIAgent::session_id_in_argv` (+ test) | which agents take a prompt on resume; read Claude's session id off its argv |
| `crates/tty7-core/src/daemon/pane.rs` | `spawn`, after `spawn_command` | `nice::apply(pid)` on the new shell |
| `crates/tty7-core/src/daemon/pane.rs` | `apply_agent` → `adopt_argv_session`, `PaneState::argv_session_miss` (+ test) | `claude_background::adopt_argv_session`, so Claude resumes without hooks |
| `crates/tty7-core/src/daemon/mod.rs` | module list | `pub(crate) mod nice`, `pub mod procstat` |
| `crates/tty7-core/Cargo.toml` | `windows-sys` features | `Win32_System_ProcessStatus` for `procstat`'s working set |
| `crates/tty7-core/src/daemon/protocol.rs` | `ProcEntry` | `rss`, `cpu_ns`, `started` (serde default), `Default` derive |
| `crates/tty7-core/src/daemon/procinfo.rs` | `snapshot`, `walk` | `procstat::fill` on the trimmed list; `..Default::default()` in `ProcEntry` literals |
| `crates/tty7-core/src/host/server.rs`, `crates/tty7-cli/src/commands.rs` | test `ProcEntry` literals | `..Default::default()` |
| `crates/tty7-cli/src/output.rs` | `procs_tables` (+ test) | RSS column appended |
| `src/ui/right_panel.rs` | `RightPanelState::cpu`, `procs_section`, `spawn_procs_query` | sample CPU on each poll; `proc_usage` cells per row and the Total line |
| `src/ui/mod.rs`, `src/ui/i18n/{mod,en,zh,ja}.rs` | module list, `PanelProcessesTotal` | `proc_usage`; "Total" |
| `crates/tty7-core/src/core/mod.rs` | module list | `pub mod history_search`, `pub mod claude_background`, `pub mod fork_config`, `pub mod fork_host`, `pub mod history_cache` |
| `crates/tty7-core/src/core/agent_history.rs` | `Found`, `claude_files`, `codex_files`, `codex_not_the_users`, `strip_injected`, `unix` made `pub(crate)` | `history_search` walks and filters the same files |
| `crates/tty7-core/src/host/mod.rs`, `host/local.rs` | `Host::fork` (default `NoFork`; the local host is its own `ForkHost`) | the fork's host calls (`fork_host`), so the UI never reads the history files |
| `src/ui/github/mod.rs` | `GitHubPanelState::session`, `github_refresh`, `off_ui` made `pub(crate)` | the Session tab's state (on by default) and mentions cache; refresh marks it due; the lookup runs on `off_ui` |
| `crates/tty7-core/src/core/github/model.rs` | `StateFilter::All`, `StateFilter::admits` | the Open \| Closed \| All switch; which rows the Session list keeps |
| `crates/tty7-core/src/core/github/api.rs` | `item` | one issue or pull request as a list row (`/issues/N`), for a mention no list page has |
| `src/ui/panel_github.rs` | test `the_tab_lists_and_opens_an_issue_without_touching_the_network`, test imports | `github_panel_prefer_origin = false`: it checks upstream's own remote order; `session.tab = false`: it checks the Issues list |
| `src/ui/github/mod.rs` | `github_target_for` | `github_session::remote_pick`: `origin` before `upstream` unless the user picked one |
| `src/ui/panel_github.rs` | `render_panel_github` (list ensure, body), `github_switch_row` (kind cells, states), `switch_cell` / `github_item_row` made `pub(crate)`, `host.clone()` into `github_branch_pull` | no kind list fetched on the Session tab; `github_session_tab_body` before the plain list; the Session cell before Issues \| Pull Requests, which light only off it; All after Open \| Closed |
| `src/main.rs` | `main`, arg scan and after `announce_detached_at_launch` | `agent_resume::wake_launch_window`: `--continue`, else `resume_agents_on_launch` |
| `src/ui/app.rs` | `Tty7App` fields + `with_session_at` init | `continue_when_tabs_land` |
| `src/main.rs` | `main`, after `keymap::init` | `hotkey_window::init` |
| `src/ui/settings/pages.rs` | `render_settings_appearance`, after the window section | `hotkey_window_settings` rows |
| `src/ui/settings/pages.rs` | `render_settings_general`, Startup & Restore group | chain `resume_agents_setting` (Resume agents on restart) |
| `src/ui/app.rs` | `quit_stop_sessions`, `restart_daemon` | body from `agent_resume::quit_stop_body` / `restart_body`, which say whether agents resume; `agent_resume::arm_restart_wake` before a confirmed restart |
| `src/ui/i18n/{mod,en,zh,ja}.rs` | after `SettingsHotkeyFadeDesc` | `SettingsResumeAgents`(`Desc`), `QuitStopServerBodyResume`, `AppRestartServerBodyResume`, `ProgramNotesDropped` |
| `Cargo.toml` | macOS deps | `raw-window-handle`, for the hotkey window's NSWindow; `block2`, for its AppKit notification observers |
| `src/ui/windows.rs` | `WindowRegistry::most_recent`, `most_recent_local` | skip `hotkey_window::workspace`, so the Dock, the tray and the CLI never land in the hotkey window |
| `.github/scripts/check-host-boundary.sh` | `ALLOW` | `hotkey_window/appkit.rs` reads its saved workspace id from the local config dir |
| `src/core/session.rs` | `WorkspaceStore::restore_one` | `hotkey_window::to_restore`: a launch or a Dock click never reopens the hotkey window as a plain one |
| `src/ui/switcher.rs` | `row_menu` (one line after the workspace verbs), `render_row` (after the slot number) | `hotkey_window::menu_item` (Set as / Unset Hotkey Workspace) and `row_badge` (the chord's keycaps on the hotkey workspace's row) |
| `src/ui/theme.rs` | `window_menu_items` | `hotkey_window::menu_label`: the chord after the hotkey workspace in the Workspaces menu |
| `src/ui/app.rs` | `adopt_workspace` | run a launch wake (`--continue`, `resume_agents_on_launch`) that arrived before the tabs did, with its prompt |
| `src/ui/app.rs` | `new_tab` | open Search Everywhere's New Tab tab when `new_tab_page` is on |
| `src/ui/app.rs` | `land_pane`, `session_to_pane` | type a resume or `run_on_land` line through `agent_resume::AtPrompt`, which asks `Host::resume_plan` and types at the first prompt |
| `src/ui/agent_launch.rs` | `run_when_ready` → `type_at_first_prompt` (+ tests) | upstream's first-prompt wait as one helper every typed launch and resume goes through, waiting `agent_resume::prompt_patience` (30s for a shell with integration, 3s without) instead of `PROMPT_WAIT` |
| `crates/tty7-core/src/daemon/pane.rs`, `shell_integration.rs` | `integrates` (+ test) | whether a spawn gets integration, from the daemon's own shell choice, so the wait knows a prompt is coming |
| `src/ui/app.rs` | `render` | `on_action` for `ContinueAllAgents`, `SearchAgents` (→ `palette::open_palette_on`) |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::ContinueAllAgents`, `CommandKind::SearchAgents` |
| `src/ui/app.rs` | `search_catalog` | `catalog.open_agent_sessions = self.open_agent_session_ids(cx)` |
| `src/ui/app.rs` | `agent_resume_command`, `session_to_pane` (connecting leaf) | return an `agent_resume::Resume` (agent, id, argv) instead of a command line; its prompt is `agent_resume::wake_prompt`, which a connecting pane carries in `PendingSpawn::agent_prompt` |
| `src/ui/app.rs` | `tabs_from_session` | the sleep test is one call, `agent_resume::record_restores_asleep` (hibernated, or with `restore_asleep` a local tab with no live pane, recorded so launch and restart wakes pick only those) |
| `src/ui/app.rs` | `PendingSpawn` literals | `..Default::default()` |
| `src/ui/agent_launch.rs` | `with_minted_session`, `launch_agent` split into `launch_agent_in` / `start_agent_in` (+ test) | mint Claude's `--session-id` at launch; launch into an explicit cwd for the new tab page |
| `src/ui/pending_pane.rs` | `PendingSpawn::agent_prompt`, `Default` derive | carry the prompt until a connecting pane lands; literals fill the rest with `..Default::default()` |
| `src/ui/diff_overlay.rs`, `src/ui/document_column.rs` | test `PendingSpawn` literals | `..Default::default()` |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` group header | width budget for, and the child, `group_color::swatch`; the `hue_slot` (section index, `None` for Ungrouped) passed to it, `header_style` and `decorate_block` |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` section loop, `header_git` after `shared_git`; `SharedGit` `pub(crate)` | `group_header::header_git`: the header names the repo's default branch, not the rows' checkout |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` group header bar | `group_header::decorate`, `group_header::chevron` on every header, its `backing` under the hover buttons |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` section loop, after `folded` | `group_header::openness`; rows stay drawn until a fold's slide ends; `rows_h` summed per row |
| `src/ui/tab_sidebar.rs` | test `folding_a_group_takes_its_rows_off_the_sidebar` | `animations = false`: it checks where a fold ends, not its slide |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` group block | `group_header::decorate_block` (whole-group fill), `group_header::clip_rows` round the rows |
| `src/ui/tab_sidebar.rs` | `row_metrics::header_budget` doc | the chevron is always drawn now |
| `src/ui/settings/pages.rs` | `render_tabs_group` | chain `group_header_settings` rows |
| `src/ui/tab_sidebar.rs` | `new_tab_in_group` | `new_tab_with_shell(None, ..)` so a group's New Tab skips the page |
| `src/ui/mod.rs` | module list | `agent_resume`, `background_tab`, `github_session`, `group_color`, `group_header`, `hotkey_window`, `new_tab_page`, `palette` |
| `src/core/actions.rs` | actions list | `ContinueAllAgents`, `SearchAgents` |
| `src/ui/keymap.rs` | `shipped_bindings`, `authored_entry`, `make_binding` | `ContinueAllAgents`; `SearchAgents` on ⌘K, so `ClearScrollback` moves to ⌘⇧K (macOS) |
| `src/ui/palette.rs` | imports; `chord_tab` (was `is_palette_chord`) and its two call sites, the new tab page's keys in `intercept`, `open_palette_on` (+ tests) | upstream's since #1026, for ⌘P; the fork adds ⌘T (New Tab) and ⌘K (Agents) as tabs the same chord logic opens, and the new tab page's own keys ahead of the modal rule |
| `src/ui/keymap.rs` | `init`; `fixed_bindings` ⌘K ⌘D comment | `background_tab::bind_shift_enter` first, so the base snapshot keeps it; the palette takes ⌘K first on macOS |
| `src/ui/settings_window.rs` | `SettingsWindow::app` | `pub(crate)`, so a palette chord in Settings opens the palette over its workspace |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `ContinueAllAgents`, `SearchAgents` in Search Everywhere |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `SearchText` (Search Text in Files…) |
| `src/ui/search/mod.rs` | `SearchTab`, `ORDER`, `title`, `placeholder`, module list, `CARD_MAX_W` re-export (+ test) | the `Text` tab, on the row between Hosts and Actions; the `History` tab, after Sessions; the `Agents` and `NewTab` tabs, off the row |
| `src/ui/search/sources.rs` | `Catalog` fields, `new`, `source`, `all` | `live` (`text::LiveTab`s), `open_agent_sessions`; `all` leaves Text and History out; New Tab has no rows of its own; `rank`, `by_section` are `pub(super)` for Agents |
| `src/ui/search/view.rs` | `perform_search`, `set_tab`, `render_empty`, `update_catalog` visibility | `Catalog::ask_live` for the tab showing; the too-short and remote hints |
| `src/ui/search/view.rs` | `SearchView::new_tab`, `set_new_tab`, `new_tab_page`, `focus`; `render` card; `CARD_MAX_W` `pub(crate)` | the New Tab tab draws `NewTabPage` in place of the list, on the palette's own card width; `palette` puts focus back in the field |
| `src/ui/panel_search.rs` | module list | `pub(crate) mod model` for `split_relative` |
| `src/ui/app.rs` | `open_search` | `catalog.live = palette_live_tabs(..)` |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::SearchText` |
| `src/ui/app.rs` | `Tty7App::open_in_background` field + init; `new_tab_slot` → `seat_new_tab`; `new_tab_insert_at` made `pub(crate)` | insert without activating inside `in_background` |
| `src/ui/app.rs` | `run_command` `LaunchAgent`, `ResumeSession`, `ForkSession` | wrap in `in_background` when ⇧ is held |
| `src/ui/search/view.rs` | `render_footer` | the ⇧↵ footer hint on tab-opening rows |
| `src/ui/i18n/mod.rs` | `L10nKey` | `CmdContinueAllAgents*`, `NewTabPage*`, `SettingsGroup*`, `CmdSearchText`, `SearchTabText`, `SearchPlaceholderText`, `SearchTextTooShort`, `SearchTabHistory`, `SearchPlaceholderHistory`, `SearchHistory*`, `CmdSearchAgents`, `SearchTabAgents`, `SearchPlaceholderAgents`, `GitHubSession`, `GitHubAll`, `GitHubNoSessionMentions`, `GitHubNoSessionMatches`, `SearchHintBackground`, `SettingsHotkey*`, `Switcher{Set,Unset}HotkeyWorkspace`, `SwitcherHotkeyWorkspace` |
| `src/ui/i18n/en.rs`, `zh.rs`, `ja.rs` | `translate_*` | those keys; `QuitStopServerBody` says tabs come back asleep |
| `docs/agents/sessions.mdx` | resume section | restore asleep, Continue All Agents, `--continue`, hook-free Claude resume |
| `docs/reference/configuration.mdx` | config table | the fork's config fields |
| `docs/window/sidebar.mdx` | Group colours | `group_colors`, header outline/fill, default branch |
| `docs/window/side-panel.mdx` | GitHub list bullets | the Session tab, All |
| `docs/window/search-everywhere.mdx` | intro; Tabs table; Sessions | ⌘T/⌘P/⌘K from anywhere, modal; the Text, History, Agents and New Tab tabs; ⇧ opens in the background |
| `docs/reference/keyboard-shortcuts.mdx` | New Tab, Search Everywhere, Clear Scrollback rows; after View | ⌘K is Agents; Clear Scrollback moved to ⌘⇧K; ⌘T is the palette's New Tab tab; the three chords work from anywhere and the palette is modal |
| `docs/docs.json` | "The window" pages | `window/new-tab-page`, `window/hotkey-window` |
| `crates/tty7-core/src/core/term_modes.rs` | `TRACKED`, `COLOR_SCHEME_UPDATES`, `feed`'s `n` and OSC arms, `osc_end`, `take_color_scheme_queries` (+ tests) | track 2031 (a reattach restores it), count `?996n` queries, and drop 2031 at a shell prompt (OSC 133 `A`/`B`/`D`), so a dead program's 2031 never sends a 997 into the shell |
| `crates/tty7-core/src/daemon/pane.rs` | `INPUT_MODE_RESETS` | `?2031l`: a restored pane's new shell never asked for theme reports |
| `src/terminal/remote.rs` | `ReaderSignals` and `RemoteTerminal` (one `fork` field each, and in their literals), `mod fork_pane`, `spawn_reader`: `fork.reader()`, `flush_batch` (`on_output` in place of the direct `notify_desktop`), after each frame decodes (`on_frame`), `Snapshot` arm (`on_replay`) | program notes go to the view, not straight to the desktop; 2031 folded and reported (see `remote/fork_pane.rs`) |
| `src/ui/app.rs`, `src/ui/tab_sidebar.rs` | `Tty7App::sidebar_reveal` (by `TabId`), `activate`; `tab_sidebar` row canvas and the undrawn-reveal clear, `reveal_shift` (+ test) | selecting a tab scrolls its row in only when it is out of view, at the nearest edge; identical to upstream/sidebar-scroll-nearest |
| `src/terminal/view.rs` | `with_terminal` (made `pub(super)` for the test) | `color_scheme::watch` |
| `src/terminal/mod.rs` | module list | `mod color_scheme` |
