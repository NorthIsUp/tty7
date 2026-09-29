# The NorthIsUp tty7 fork

This is [l0ng-ai/tty7](https://github.com/l0ng-ai/tty7) plus agent resume after
a reboot (Continue All Agents), a new tab page, a Text (find in files) tab and a History (agent conversations) tab in
Search Everywhere, sidebar group colours and a niceness for pane shells. It stays rebasable by one rule: fork logic lives in
fork-owned files, and an upstream file gets only a short hook call into them,
listed below. `mise run sync-upstream` rebases onto `upstream/main`; when it
stops on a conflict, the hook table says what each fork hunk in that file is
for, so keep upstream's side, re-add the hook, and `git rebase --continue`.

## The app

`mise run install-app` builds the fast profile into
`~/Applications/tty7-niu-dev.app` (`com.northisup.tty7-niu-dev`), signed with
the keychain's Developer ID Application identity (or `TTY7_SIGN_ID`). The
designated requirement names the bundle id and team, not the build, so macOS
privacy grants survive every reinstall. The running app watches the bundle's
`local-build-id` and offers a restart when a new build lands; it never checks
GitHub. `mise run launch` and `reload` run that bundle.

CI (`release.yml`, on a `v*` tag) ships `tty7-niu.app` (`com.northisup.tty7-niu`),
notarized, and publishes the release here; its updater reads this repo's
releases. The signing secrets come from `! mise run set-release-secrets`.

## Fork-owned files

| file | what it holds |
|---|---|
| `FORK.md` | this page |
| `mise.toml`, `mise-tasks/` | tool pin; build, test, release and `sync-upstream`; `build-fast`, `install-app` (the fast build as `~/Applications/tty7-niu-dev.app`, signed), `launch` (that bundle, clean env) and `reload` (window only, refuses when the server's code changed); `set-release-secrets` (human-run, release.yml's signing secrets) |
| `src/core/fork_update.rs` | the update feed's repo (`update_repo!`, NorthIsUp/tty7); a local install checks no feed and prompts to restart when `install-app` lands a new build |
| `docs/fork/**` | spec, master plan and task plans for the fork |
| `docs/window/new-tab-page.mdx` | user docs for the new tab page |
| `src/ui/agent_resume.rs` | Continue All Agents, `--continue`, which dead tabs restore asleep |
| `src/ui/new_tab_page.rs` | the new tab page picker (agent or terminal, and a directory) |
| `src/ui/search/text.rs` | Search Everywhere's Text tab: find in files over `Host::search_content`, debounced, never on All; the debounce and query plumbing History shares |
| `src/ui/search/agents.rs` | Search Everywhere's Agents tab (⌘K): the Terminals tab's open tabs, then the Sessions tab's rows not open in any pane |
| `src/ui/search/history_text.rs` | Search Everywhere's History tab: full text over past agent conversations, one row per session, Enter resumes it |
| `crates/tty7-core/src/core/history_search.rs` | the History tab's scan: Claude, Qoder and Codex transcripts streamed newest first, what was said cached by path and mtime |
| `src/ui/group_color.rs` | a group's colour (override, else hashed into the theme) and its swatch |
| `src/ui/group_header.rs` | a group header's outline and fill (header or whole group), its chevron, the fold slide, the repo default branch it names, and their Settings rows |
| `crates/tty7-core/src/daemon/nice.rs` | `setpriority` on a pane's shell from `Config::nice` |
| `crates/tty7-core/src/daemon/procstat.rs` | per-process RSS, CPU time and start stamp for Info → Processes; `compact_bytes` |
| `src/ui/proc_usage.rs` | CPU% from two samples, the Processes row's CPU / memory / pid cells and its Total line |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `Cargo.toml` | `[profile.fast]` | the day-to-day build: deps at opt 3, the app crate at opt 1, no LTO |
| `src/core/update.rs` | `REPO`, `RELEASES_URL`, `NIGHTLY_RELEASE_URL` | `update_repo!()`, so checks and links read the fork's releases |
| `src/core/update.rs` | `spawn_check`, `spawn_check_inner` | `fork_update::watch`; a local install skips the GitHub check |
| `src/core/mod.rs` | module list | `pub mod fork_update` |
| `src/bin/tty7-updater.rs` | `install_inner`, `extract_archive` → `unpacked_app` (+ test) | find the unpacked `.app` rather than name `tty7.app`, since the fork's is `tty7-niu.app` |
| `.github/scripts/bundle-macos.sh` | top, Info.plist, signing, notarization, after the sweep | `TTY7_APP_NAME`, `TTY7_BUNDLE_ID`, `TTY7_BIN_DIR`, `TTY7_DIST`, `TTY7_LOCAL_BUILD_ID`, `TTY7_BUNDLE_ONLY`; sign with a keychain identity when no cert is imported (no timestamp); notarize with an ASC API key (`ASC_KEY_P8`, `ASC_KEY_ID`, `ASC_ISSUER_ID`) |
| `.github/workflows/release.yml` | `Bundle macOS DMG` env; `draft-release` last step | build `tty7-niu.app` (`com.northisup.tty7-niu`) with the ASC notarization key; publish the draft on NorthIsUp/tty7 |
| `crates/tty7-core/src/core/config.rs` | `Config` fields, `Default`, `default_*` fns | `restore_asleep`, `continue_prompt`, `continue_stagger_ms`, `resume_agents_on_launch`, `new_tab_page`, `dir_roots`, `dir_frecency`, `group_colors`, `group_outline`, `group_background`, `group_outline_color`, `group_background_color`, `group_background_scope`, `animations`, `nice`; `GroupColorSource`, `GroupBackgroundScope` |
| `crates/tty7-core/src/core/cli_agent.rs` | `CLIAgent::resume_takes_prompt`, `CLIAgent::session_id_in_argv` (+ test) | which agents take a prompt on resume; read Claude's session id off its argv |
| `crates/tty7-core/src/daemon/pane.rs` | `spawn`, after `spawn_command` | `nice::apply(pid)` on the new shell |
| `crates/tty7-core/src/daemon/pane.rs` | `apply_agent` → new `adopt_argv_session` (+ test) | adopt the argv's session id so Claude resumes without hooks |
| `crates/tty7-core/src/daemon/mod.rs` | module list | `pub(crate) mod nice`, `pub mod procstat` |
| `crates/tty7-core/Cargo.toml` | `windows-sys` features | `Win32_System_ProcessStatus` for `procstat`'s working set |
| `crates/tty7-core/src/daemon/protocol.rs` | `ProcEntry` | `rss`, `cpu_ns`, `started` (serde default), `Default` derive |
| `crates/tty7-core/src/daemon/procinfo.rs` | `snapshot`, `walk` | `procstat::fill` on the trimmed list; `..Default::default()` in `ProcEntry` literals |
| `crates/tty7-core/src/host/server.rs`, `crates/tty7-cli/src/commands.rs` | test `ProcEntry` literals | `..Default::default()` |
| `crates/tty7-cli/src/output.rs` | `procs_tables` (+ test) | RSS column appended |
| `src/ui/right_panel.rs` | `RightPanelState::cpu`, `procs_section`, `spawn_procs_query` | sample CPU on each poll; `proc_usage` cells per row and the Total line |
| `src/ui/mod.rs`, `src/ui/i18n/{mod,en,zh,ja}.rs` | module list, `PanelProcessesTotal` | `proc_usage`; "Total" |
| `crates/tty7-core/src/core/mod.rs` | module list | `pub mod history_search` |
| `crates/tty7-core/src/core/agent_history.rs` | `Found`, `claude_files`, `codex_files`, `codex_not_the_users`, `strip_injected`, `unix` made `pub(crate)` | `history_search` walks and filters the same files |
| `crates/tty7-core/src/host/mod.rs`, `host/local.rs` | `Host::search_agent_history` (default empty; the local host runs `history_search::search`) | History searches through `Host`, so the UI never reads files |
| `src/main.rs` | `main`, arg scan and after `announce_detached_at_launch` | `agent_resume::wake_launch_window`: `--continue`, else `resume_agents_on_launch` |
| `src/ui/app.rs` | `Tty7App` fields + `with_session_at` init | `continue_when_tabs_land`, `new_tab_page` |
| `src/ui/app.rs` | `with_session_at`, `on_focus_lost` → `focus_active`; test mod `unfocused_shortcut_tests` | focus left on nothing, or on a handle no element draws, dispatches keys on the window root only, above every `tty7-root` listener, so ⌘P and the rest went dead |
| `src/ui/sftp.rs` | test `cancelling_the_edit_form_hands_focus_back` | checks focus with no frame in between, since a frame now hands focus on an undrawn box back to the app |
| `src/ui/app.rs` | `adopt_workspace` | run a launch wake (`--continue`, `resume_agents_on_launch`) that arrived before the tabs did, with its prompt |
| `src/ui/app.rs` | `new_tab` | open the new tab page when `new_tab_page` is on |
| `src/ui/app.rs` | `land_pane`, `session_to_pane` | type a resume through `run_at_prompt`, not ahead of the shell's startup |
| `src/ui/agent_launch.rs` | `run_when_ready` | same, for a quick-launched agent |
| `src/terminal/view.rs` | `TerminalView` field + `run_at_prompt` + `poll_foreground` | hold a line until the shell's first prompt; startup files that read the terminal swallow typeahead |
| `src/ui/app.rs` | `render` | `render_new_tab_page` child; `on_action` for `ContinueAllAgents`, `SearchAgents` |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::ContinueAllAgents`, `CommandKind::SearchAgents` |
| `src/ui/app.rs` | `search_catalog` | `catalog.open_agent_sessions = self.open_agent_session_ids(cx)` |
| `src/ui/app.rs` | `wake_tab` → `wake_tab_with` | wake with a prompt for the resumed agent |
| `src/ui/app.rs` | `agent_resume_command`, `session_to_pane`, `land_pane`, `reopen_closed_tab` | thread `prompt` through to the resume command line |
| `src/ui/app.rs` | `tabs_from_session` | restore a tab with no live pane asleep (`restore_asleep`) |
| `src/ui/app.rs` | test `PendingSpawn` literals | `agent_prompt: None` |
| `src/ui/agent_launch.rs` | `with_minted_session`, `launch_agent` split into `launch_agent_in` / `start_agent_in` (+ test) | mint Claude's `--session-id` at launch; launch into an explicit cwd for the new tab page |
| `src/ui/pending_pane.rs` | `PendingSpawn::agent_prompt` | carry the prompt until a connecting pane lands |
| `src/ui/diff_overlay.rs`, `src/ui/document_column.rs` | `PendingSpawn` literals | `agent_prompt: None` |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` group header | width budget for, and the child, `group_color::swatch` |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` section loop, `header_git` after `shared_git` | the header names `group_header::default_branch`, not the rows' checkout |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` group header bar | `group_header::decorate`, `group_header::chevron` on every header, its `backing` under the hover buttons |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` section loop, after `folded` | `group_header::openness`; rows stay drawn until a fold's slide ends; `rows_h` summed per row |
| `src/ui/tab_sidebar.rs` | test `folding_a_group_takes_its_rows_off_the_sidebar` | `animations = false`: it checks where a fold ends, not its slide |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` group block | `group_header::decorate_block` (whole-group fill), `group_header::clip_rows` round the rows |
| `src/ui/tab_sidebar.rs` | `row_metrics::header_budget` doc | the chevron is always drawn now |
| `src/ui/settings/pages.rs` | `render_tabs_group` | chain `group_header_settings` rows |
| `src/ui/tab_sidebar.rs` | `new_tab_in_group` | `new_tab_with_shell(None, ..)` so a group's New Tab skips the page |
| `src/ui/mod.rs` | module list | `agent_resume`, `group_color`, `group_header`, `new_tab_page` |
| `src/core/actions.rs` | actions list | `ContinueAllAgents`, `SearchAgents`; `NewTabPageNextKind`, `NewTabPagePrevKind` (Tab on the new tab page) |
| `src/ui/keymap.rs` | `shipped_bindings`, `authored_entry`, `make_binding` | `ContinueAllAgents`; `SearchAgents` on ⌘K, so `ClearScrollback` moves to ⌘⇧K (macOS) |
| `src/ui/keymap.rs` | `fixed_bindings` | Tab / ⇧Tab bound in the `NewTabPage` context, since Root's focus walker otherwise takes Tab |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `ContinueAllAgents`, `SearchAgents` in Search Everywhere |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `SearchText` (Search Text in Files…) |
| `src/ui/search/mod.rs` | `SearchTab`, `ORDER`, `title`, `placeholder`, module list (+ test) | the `Text` tab, on the row between Hosts and Actions; the `History` tab, after Sessions; the `Agents` tab, off the row |
| `src/ui/search/sources.rs` | `Catalog` fields, `new`, `source`, `all` | `text`, `text_query`, `history`, `history_query`, `open_agent_sessions`; `all` leaves Text and History out; `rank`, `by_section` are `pub(super)` for Agents |
| `src/ui/search/view.rs` | `perform_search`, `set_tab`, `render_empty`, `update_catalog` and `match_range` visibility, `text_rows` (test) | ask the window for text and history hits; the too-short and remote hints |
| `src/ui/panel_search.rs` | module list | `pub(crate) mod model` for `split_relative` |
| `src/ui/app.rs` | `open_search` | `catalog.text_query = palette_text_query(..)`, `catalog.history_query = palette_history_query(..)` |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::SearchText` |
| `src/ui/i18n/mod.rs` | `L10nKey` | `CmdContinueAllAgents*`, `NewTabPage*`, `SettingsGroup*`, `CmdSearchText`, `SearchTabText`, `SearchPlaceholderText`, `SearchTextTooShort`, `SearchTabHistory`, `SearchPlaceholderHistory`, `SearchHistory*`, `CmdSearchAgents`, `SearchTabAgents`, `SearchPlaceholderAgents` |
| `src/ui/i18n/en.rs`, `zh.rs`, `ja.rs` | `translate_*` | those keys; `QuitStopServerBody` says tabs come back asleep |
| `docs/agents/sessions.mdx` | resume section | restore asleep, Continue All Agents, `--continue`, hook-free Claude resume |
| `docs/reference/configuration.mdx` | config table | the fork's config fields |
| `docs/window/sidebar.mdx` | Group colours | `group_colors`, header outline/fill, default branch |
| `docs/window/search-everywhere.mdx` | Tabs table | the Text, History and Agents tabs |
| `docs/reference/keyboard-shortcuts.mdx` | Search Everywhere, Clear Scrollback rows | ⌘K is Agents; Clear Scrollback moved to ⌘⇧K |
| `docs/docs.json` | "The window" pages | `window/new-tab-page` |
| `.github/workflows/ci.yml` | `changes` job; `needs`/`if` on `build` steps and the server jobs; `build` env; the three `Swatinem/rust-cache` steps | skip the Rust jobs on docs-only PRs while required checks still report; save caches from main and manual runs only, keep them on failure, build tests with `line-tables-only` debug so the cache is smaller |
