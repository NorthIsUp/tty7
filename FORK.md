# The NorthIsUp tty7 fork

This is [l0ng-ai/tty7](https://github.com/l0ng-ai/tty7) plus agent resume after
a reboot (Continue All Agents), a new tab page, a Text (find in files) tab and a History (agent conversations) tab in
Search Everywhere, sidebar group colours and a niceness for pane shells. It stays rebasable by one rule: fork logic lives in
fork-owned files, and an upstream file gets only a short hook call into them,
listed below. The fork's work lives on `main-niu` (the default branch); `main`
mirrors upstream and never gets fork commits. `mise run sync-upstream` syncs
`main` from upstream and merges `upstream/main` into the current branch; when it
stops on a conflict, the hook table says what each fork hunk in that file is
for, so keep upstream's side, re-add the hook, and `git commit`.

## The app

`mise run install-app` builds the fast profile into
`~/Applications/tty7-niu-dev.app` (`com.northisup.tty7-niu-dev`), signed with
the keychain's Developer ID Application identity (or `TTY7_SIGN_ID`). The
designated requirement names the bundle id and team, not the build, so macOS
privacy grants survive every reinstall. The running app watches the bundle's
`local-build-id` and offers a restart when a new build lands; it never checks
GitHub. `mise run launch` and `reload` run that bundle.

A merge to main that bumps the workspace `version` in `Cargo.toml` gets tagged
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
| `src/ui/agent_resume.rs` | Continue All Agents, `--continue`, which dead tabs restore asleep |
| `src/ui/new_tab_page.rs` | the new tab page picker (agent or terminal, and a directory), drawn as Search Everywhere's New Tab tab |
| `src/ui/palette.rs` | ⌘T / ⌘P / ⌘K open Search Everywhere on New Tab / All / Agents wherever focus is (the Settings window too), and an open palette keeps every key: a keystroke interceptor, ahead of all bindings |
| `src/ui/background_tab.rs` | ⇧ opens a tab in the background: `in_background`, `seat_new_tab`, which palette rows take it, where `active` lands |
| `src/ui/search/text.rs` | Search Everywhere's Text tab: find in files over `Host::search_content`, debounced, never on All; the debounce and query plumbing History shares |
| `src/ui/search/agents.rs` | Search Everywhere's Agents tab (⌘K): the Terminals tab's open tabs, then the Sessions tab's rows not open in any pane |
| `src/ui/search/history_text.rs` | Search Everywhere's History tab: full text over past agent conversations, one row per session, Enter resumes it |
| `crates/tty7-core/src/core/history_search.rs` | the History tab's scan: Claude, Qoder and Codex transcripts streamed newest first, what was said cached by path and mtime; `session_mentions`, one session's issue and PR references, tool output included; a bare `#N` only when the session ran in a checkout of the shown repo |
| `.github/workflows/tag-on-bump.yml` | on a main push that bumps the workspace version: tag `v<version>` and dispatch `release.yml` on it |
| `src/ui/group_color.rs` | a group's colour (override, else a golden-angle hue by sidebar place; Ungrouped grey) and its swatch |
| `src/ui/group_header.rs` | a group header's outline and fill (header or whole group), its chevron, the fold slide, the repo default branch it names, and their Settings rows |
| `src/ui/github_session.rs` | the GitHub tab's "This session" filter: Pull Requests narrowed to those the focused pane's agent session mentions, the rest as `#N` chips; `remote_pick`, the fork before upstream; `panel_state`, the list it opens on |
| `crates/tty7-core/src/daemon/nice.rs` | `setpriority` on a pane's shell from `Config::nice` |
| `crates/tty7-core/src/daemon/procstat.rs` | per-process RSS, CPU time and start stamp for Info → Processes; `compact_bytes` |
| `src/ui/proc_usage.rs` | CPU% from two samples, the Processes row's CPU / memory / pid cells and its Total line |
| `src/terminal/element/osc8_underline.rs` | an OSC 8 link's resting faint dotted underline (iTerm2's), solid under the pointer; an SGR underline keeps its own |
| `crates/tty7-core/src/core/claude_background.rs` | Claude sessions running in the background: session ↔ job id from `sessions/<pid>.json`, and a resume line turned into `claude attach <job>` |
| `src/ui/hotkey_window.rs` | the global hotkey (`global_hotkey`, ⌥Space): Carbon `RegisterEventHotKey`, one dedicated hotkey window (its workspace saved in `hotkey-window`, picked from a switcher row's menu) shown / focused / ordered out with a fade, the full screen modal, windows activated over it lifted above it, hide on focus loss, and its Settings rows (macOS; a no-op elsewhere) |
| `docs/window/hotkey-window.mdx` | user docs for the hotkey window |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/daemon/pane.rs` | `pane_environment` | `FORCE_HYPERLINK=1`: `supports-hyperlinks` (Claude Code's statusline, Node CLIs) strips OSC 8 for a TERM_PROGRAM it doesn't know |
| `src/terminal/element.rs` | `mod osc8_underline`, `RenderCell::osc8_dots`, end of `snapshot_cell`, `flag_hovered_link`; test `test_colors` made `pub(super)` | `osc8_underline::mark` / `unmark` |
| `Cargo.toml` | `[profile.fast]` | the day-to-day build: deps at opt 3, the app crate at opt 1, no LTO |
| `src/core/update.rs` | `REPO`, `RELEASES_URL`, `NIGHTLY_RELEASE_URL` | `update_repo!()`, so checks and links read the fork's releases |
| `src/core/update.rs` | `spawn_check`, `spawn_check_inner` | `fork_update::watch`; a local install skips the GitHub check |
| `src/core/mod.rs` | module list | `pub mod fork_update` |
| `src/bin/tty7-updater.rs` | `install_inner`, `extract_archive` → `unpacked_app` (+ test) | find the unpacked `.app` rather than name `tty7.app`, since the fork's is `tty7-niu.app` |
| `.github/scripts/bundle-macos.sh` | top, Info.plist, signing, notarization, after the sweep | `TTY7_APP_NAME`, `TTY7_BUNDLE_ID`, `TTY7_BIN_DIR`, `TTY7_DIST`, `TTY7_LOCAL_BUILD_ID`, `TTY7_BUNDLE_ONLY`; sign with a keychain identity when no cert is imported (no timestamp); notarize with an ASC API key (`ASC_KEY_P8`, `ASC_KEY_ID`, `ASC_ISSUER_ID`) |
| `.github/workflows/release.yml` | `Bundle macOS DMG` env; `draft-release` last step | build `tty7-niu.app` (`com.northisup.tty7-niu`) with the ASC notarization key; publish the draft on NorthIsUp/tty7 |
| `crates/tty7-core/src/core/config.rs` | `Config` fields, `Default`, `default_*` fns | `restore_asleep`, `continue_prompt`, `continue_stagger_ms`, `resume_agents_on_launch`, `new_tab_page`, `dir_roots`, `dir_frecency`, `group_colors`, `group_outline`, `group_background`, `group_outline_color`, `group_background_color`, `group_background_scope`, `animations`, `nice`, `github_panel_session_filter`, `github_panel_prefer_origin`, `github_panel_default_list`, `global_hotkey`, `global_hotkey_fullscreen`, `global_hotkey_hide_on_blur`, `global_hotkey_fade_ms`; `GroupColorSource`, `GroupBackgroundScope` |
| `crates/tty7-core/src/core/cli_agent.rs` | `CLIAgent::resume_takes_prompt`, `CLIAgent::session_id_in_argv` (+ test) | which agents take a prompt on resume; read Claude's session id off its argv |
| `crates/tty7-core/src/daemon/pane.rs` | `spawn`, after `spawn_command` | `nice::apply(pid)` on the new shell |
| `crates/tty7-core/src/daemon/pane.rs` | `apply_agent` → new `adopt_argv_session` (+ test) | adopt the argv's session id so Claude resumes without hooks; `claude attach <job>` maps back through `claude_background` |
| `crates/tty7-core/src/daemon/mod.rs` | module list | `pub(crate) mod nice`, `pub mod procstat` |
| `crates/tty7-core/Cargo.toml` | `windows-sys` features | `Win32_System_ProcessStatus` for `procstat`'s working set |
| `crates/tty7-core/src/daemon/protocol.rs` | `ProcEntry` | `rss`, `cpu_ns`, `started` (serde default), `Default` derive |
| `crates/tty7-core/src/daemon/procinfo.rs` | `snapshot`, `walk` | `procstat::fill` on the trimmed list; `..Default::default()` in `ProcEntry` literals |
| `crates/tty7-core/src/host/server.rs`, `crates/tty7-cli/src/commands.rs` | test `ProcEntry` literals | `..Default::default()` |
| `crates/tty7-cli/src/output.rs` | `procs_tables` (+ test) | RSS column appended |
| `src/ui/right_panel.rs` | `RightPanelState::cpu`, `procs_section`, `spawn_procs_query` | sample CPU on each poll; `proc_usage` cells per row and the Total line |
| `src/ui/mod.rs`, `src/ui/i18n/{mod,en,zh,ja}.rs` | module list, `PanelProcessesTotal` | `proc_usage`; "Total" |
| `crates/tty7-core/src/core/mod.rs` | module list | `pub mod history_search`, `pub mod claude_background` |
| `crates/tty7-core/src/core/agent_history.rs` | `Found`, `claude_files`, `codex_files`, `codex_not_the_users`, `strip_injected`, `unix` made `pub(crate)` | `history_search` walks and filters the same files |
| `crates/tty7-core/src/host/mod.rs`, `host/local.rs` | `Host::search_agent_history` (default empty; the local host runs `history_search::search`) | History searches through `Host`, so the UI never reads files |
| `crates/tty7-core/src/host/mod.rs`, `host/local.rs` | `Host::agent_session_mentions` (default empty; the local host runs `history_search::session_mentions`) | the "This session" filter reads the transcript through `Host` |
| `src/ui/github/mod.rs` | `GitHubPanelState::session`, `github_refresh` | the filter's mentions cache; refresh marks it due |
| `src/ui/panel_github.rs` | test `the_tab_lists_and_opens_an_issue_without_touching_the_network`, test imports | `github_panel_prefer_origin = false`: it checks upstream's own remote order |
| `src/ui/github/mod.rs` | `github_target_for` | `github_session::remote_pick`: `origin` before `upstream` unless the user picked one |
| `src/ui/panel_github.rs` | `render_panel_github` body, `github_switch_row` state group, `switch_cell` / `github_item_row` made `pub(crate)`, `host.clone()` into `github_branch_pull` | `github_session_body` before the plain list; the This session chip |
| `crates/tty7-core/src/host/mod.rs`, `host/local.rs` | `Host::claude_background_job` (default `None`; the local host reads `claude_background`) | the resume line checks for a background session off the UI thread |
| `src/main.rs` | `main`, arg scan and after `announce_detached_at_launch` | `agent_resume::wake_launch_window`: `--continue`, else `resume_agents_on_launch` |
| `src/ui/app.rs` | `Tty7App` fields + `with_session_at` init | `continue_when_tabs_land`; `github` from `github_session::panel_state` (`github_panel_default_list`) |
| `src/ui/app.rs` | `with_session_at`, `on_focus_lost` → `focus_active`; test mod `unfocused_shortcut_tests` | focus left on nothing, or on a handle no element draws, dispatches keys on the window root only, above every `tty7-root` listener, so ⌘P and the rest went dead |
| `src/ui/sftp.rs` | test `cancelling_the_edit_form_hands_focus_back` | checks focus with no frame in between, since a frame now hands focus on an undrawn box back to the app |
| `src/main.rs` | `main`, after `keymap::init` | `hotkey_window::init` |
| `src/ui/settings/pages.rs` | `render_settings_appearance`, after the window section | `hotkey_window_settings` rows |
| `Cargo.toml` | macOS deps | `raw-window-handle`, for the hotkey window's NSWindow; `block2`, for its AppKit notification observers |
| `src/ui/windows.rs` | `WindowRegistry::most_recent`, `most_recent_local` | skip `hotkey_window::workspace`, so the Dock, the tray and the CLI never land in the hotkey window |
| `.github/scripts/check-host-boundary.sh` | `ALLOW` | `hotkey_window.rs` reads its saved workspace id from the local config dir |
| `src/core/session.rs` | `WorkspaceStore::restore_one` | `hotkey_window::to_restore`: a launch or a Dock click never reopens the hotkey window as a plain one |
| `src/ui/switcher.rs` | `row_menu`, `render_row` (after the slot number) | `hotkey_window::menu_item` (Set as / Unset Hotkey Workspace) and `row_badge` (the chord's keycaps on the hotkey workspace's row) |
| `src/ui/theme.rs` | `window_menu_items` | `hotkey_window::menu_label`: the chord after the hotkey workspace in the Workspaces menu |
| `src/ui/app.rs` | `adopt_workspace` | run a launch wake (`--continue`, `resume_agents_on_launch`) that arrived before the tabs did, with its prompt |
| `src/ui/app.rs` | `new_tab` | open Search Everywhere's New Tab tab when `new_tab_page` is on |
| `src/ui/app.rs` | `land_pane`, `session_to_pane` | type a resume through `run_at_prompt`, not ahead of the shell's startup |
| `src/ui/agent_launch.rs` | `run_when_ready` | same, for a quick-launched agent |
| `src/terminal/view.rs` | `TerminalView` field + `run_at_prompt` (takes `cx`) + `queue_at_prompt` + `poll_foreground` | hold a line until the shell's first prompt; startup files that read the terminal swallow typeahead; a resume of a background Claude session becomes `claude attach` |
| `src/ui/app.rs` | `render` | `on_action` for `ContinueAllAgents`, `SearchAgents` |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::ContinueAllAgents`, `CommandKind::SearchAgents` |
| `src/ui/app.rs` | `search_catalog` | `catalog.open_agent_sessions = self.open_agent_session_ids(cx)` |
| `src/ui/app.rs` | `wake_tab` → `wake_tab_with` | wake with a prompt for the resumed agent |
| `src/ui/app.rs` | `agent_resume_command`, `session_to_pane`, `land_pane`, `reopen_closed_tab` | thread `prompt` through to the resume command line |
| `src/ui/app.rs` | `tabs_from_session` | restore a tab with no live pane asleep (`restore_asleep`) |
| `src/ui/app.rs` | test `PendingSpawn` literals | `agent_prompt: None` |
| `src/ui/agent_launch.rs` | `with_minted_session`, `launch_agent` split into `launch_agent_in` / `start_agent_in` (+ test) | mint Claude's `--session-id` at launch; launch into an explicit cwd for the new tab page |
| `src/ui/pending_pane.rs` | `PendingSpawn::agent_prompt` | carry the prompt until a connecting pane lands |
| `src/ui/diff_overlay.rs`, `src/ui/document_column.rs` | `PendingSpawn` literals | `agent_prompt: None` |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` group header | width budget for, and the child, `group_color::swatch`; the `hue_slot` (section index, `None` for Ungrouped) passed to it, `header_style` and `decorate_block` |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` section loop, `header_git` after `shared_git` | the header names `group_header::default_branch`, not the rows' checkout |
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
| `src/ui/keymap.rs` | `init`; `fixed_bindings` ⌘K ⌘D comment | `palette::init`; the palette takes ⌘K first on macOS |
| `src/ui/settings_window.rs` | `SettingsWindow::app` | `pub(crate)`, so a palette chord in Settings opens the palette over its workspace |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `ContinueAllAgents`, `SearchAgents` in Search Everywhere |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `SearchText` (Search Text in Files…) |
| `src/ui/search/mod.rs` | `SearchTab`, `ORDER`, `title`, `placeholder`, module list (+ test) | the `Text` tab, on the row between Hosts and Actions; the `History` tab, after Sessions; the `Agents` and `NewTab` tabs, off the row |
| `src/ui/search/sources.rs` | `Catalog` fields, `new`, `source`, `all` | `text`, `text_query`, `history`, `history_query`, `open_agent_sessions`; `all` leaves Text and History out; New Tab has no rows of its own; `rank`, `by_section` are `pub(super)` for Agents |
| `src/ui/search/view.rs` | `perform_search`, `set_tab`, `render_empty`, `update_catalog` and `match_range` visibility, `text_rows` (test) | ask the window for text and history hits; the too-short and remote hints |
| `src/ui/search/view.rs` | `SearchView::new_tab`, `set_new_tab`, `new_tab_page`, `focus`; `render` card | the New Tab tab draws `NewTabPage` in place of the list; `palette` puts focus back in the field |
| `src/ui/panel_search.rs` | module list | `pub(crate) mod model` for `split_relative` |
| `src/ui/app.rs` | `open_search` | `catalog.text_query = palette_text_query(..)`, `catalog.history_query = palette_history_query(..)` |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::SearchText` |
| `src/ui/app.rs` | `Tty7App::open_in_background` field + init; `new_tab_slot` → `seat_new_tab`; `new_tab_insert_at` made `pub(crate)` | insert without activating inside `in_background` |
| `src/ui/app.rs` | `run_command` `LaunchAgent`, `ResumeSession`, `ForkSession` | wrap in `in_background` when ⇧ is held |
| `src/ui/search/view.rs` | `render` `on_key_down`, `render_footer` | ⇧Enter confirms the row (the list binds bare Enter only); the ⇧↵ footer hint on tab-opening rows |
| `src/ui/i18n/mod.rs` | `L10nKey` | `CmdContinueAllAgents*`, `NewTabPage*`, `SettingsGroup*`, `CmdSearchText`, `SearchTabText`, `SearchPlaceholderText`, `SearchTextTooShort`, `SearchTabHistory`, `SearchPlaceholderHistory`, `SearchHistory*`, `CmdSearchAgents`, `SearchTabAgents`, `SearchPlaceholderAgents`, `GitHubThisSession`, `GitHubNoSessionPulls`, `GitHubShowAllPulls`, `GitHubMoreMentioned`, `SearchHintBackground`, `SettingsHotkey*`, `Switcher{Set,Unset}HotkeyWorkspace`, `SwitcherHotkeyWorkspace` |
| `src/ui/i18n/en.rs`, `zh.rs`, `ja.rs` | `translate_*` | those keys; `QuitStopServerBody` says tabs come back asleep |
| `docs/agents/sessions.mdx` | resume section | restore asleep, Continue All Agents, `--continue`, hook-free Claude resume |
| `docs/reference/configuration.mdx` | config table | the fork's config fields |
| `docs/window/sidebar.mdx` | Group colours | `group_colors`, header outline/fill, default branch |
| `docs/window/side-panel.mdx` | GitHub list bullets | the This session filter |
| `docs/window/search-everywhere.mdx` | intro; Tabs table; Sessions | ⌘T/⌘P/⌘K from anywhere, modal; the Text, History, Agents and New Tab tabs; ⇧ opens in the background |
| `docs/reference/keyboard-shortcuts.mdx` | New Tab, Search Everywhere, Clear Scrollback rows; after View | ⌘K is Agents; Clear Scrollback moved to ⌘⇧K; ⌘T is the palette's New Tab tab; the three chords work from anywhere and the palette is modal |
| `docs/docs.json` | "The window" pages | `window/new-tab-page`, `window/hotkey-window` |
| `.github/workflows/ci.yml` | `changes` job; `needs`/`if` on `build` steps and the server jobs; `build` env; the three `Swatinem/rust-cache` steps | skip the Rust jobs on docs-only PRs while required checks still report; save caches from main and manual runs only, keep them on failure, build tests with `line-tables-only` debug so the cache is smaller |
