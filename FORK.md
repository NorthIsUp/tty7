# The NorthIsUp tty7 fork

This is [l0ng-ai/tty7](https://github.com/l0ng-ai/tty7) plus agent resume after
a reboot (Continue All Agents), a new tab page, a Text (find in files) tab in
Search Everywhere, sidebar group colours and a niceness for pane shells. It stays rebasable by one rule: fork logic lives in
fork-owned files, and an upstream file gets only a short hook call into them,
listed below. `mise run sync-upstream` rebases onto `upstream/main`; when it
stops on a conflict, the hook table says what each fork hunk in that file is
for, so keep upstream's side, re-add the hook, and `git rebase --continue`.

## Fork-owned files

| file | what it holds |
|---|---|
| `FORK.md` | this page |
| `mise.toml`, `mise-tasks/` | tool pin; build, test, release, launch and `sync-upstream` file tasks |
| `docs/fork/**` | spec, master plan and task plans for the fork |
| `docs/window/new-tab-page.mdx` | user docs for the new tab page |
| `src/ui/agent_resume.rs` | Continue All Agents, `--continue`, which dead tabs restore asleep |
| `src/ui/new_tab_page.rs` | the new tab page picker (agent or terminal, and a directory) |
| `src/ui/search/text.rs` | Search Everywhere's Text tab: find in files over `Host::search_content`, debounced, never on All |
| `src/ui/group_color.rs` | a group's colour (override, else hashed into the theme) and its swatch |
| `crates/tty7-core/src/daemon/nice.rs` | `setpriority` on a pane's shell from `Config::nice` |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/config.rs` | `Config` fields, `Default`, `default_*` fns | `restore_asleep`, `continue_prompt`, `continue_stagger_ms`, `new_tab_page`, `dir_roots`, `dir_frecency`, `group_colors`, `nice` |
| `crates/tty7-core/src/core/cli_agent.rs` | `CLIAgent::resume_takes_prompt`, `CLIAgent::session_id_in_argv` (+ test) | which agents take a prompt on resume; read Claude's session id off its argv |
| `crates/tty7-core/src/daemon/pane.rs` | `spawn`, after `spawn_command` | `nice::apply(pid)` on the new shell |
| `crates/tty7-core/src/daemon/pane.rs` | `apply_agent` → new `adopt_argv_session` (+ test) | adopt the argv's session id so Claude resumes without hooks |
| `crates/tty7-core/src/daemon/mod.rs` | module list | `pub(crate) mod nice` |
| `src/main.rs` | `main`, arg scan and after `announce_detached_at_launch` | `--continue` runs `agent_resume::continue_in_launch_window` |
| `src/ui/app.rs` | `Tty7App` fields + `with_session_at` init | `continue_when_tabs_land`, `new_tab_page` |
| `src/ui/app.rs` | `adopt_workspace` | run a `--continue` that arrived before the tabs did |
| `src/ui/app.rs` | `new_tab` | open the new tab page when `new_tab_page` is on |
| `src/ui/app.rs` | `render` | `render_new_tab_page` child; `on_action` for `ContinueAllAgents` |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::ContinueAllAgents` |
| `src/ui/app.rs` | `wake_tab` → `wake_tab_with` | wake with a prompt for the resumed agent |
| `src/ui/app.rs` | `agent_resume_command`, `session_to_pane`, `land_pane`, `reopen_closed_tab` | thread `prompt` through to the resume command line |
| `src/ui/app.rs` | `tabs_from_session` | restore a tab with no live pane asleep (`restore_asleep`) |
| `src/ui/app.rs` | test `PendingSpawn` literals | `agent_prompt: None` |
| `src/ui/agent_launch.rs` | `with_minted_session`, `launch_agent` split into `launch_agent_in` / `start_agent_in` (+ test) | mint Claude's `--session-id` at launch; launch into an explicit cwd for the new tab page |
| `src/ui/pending_pane.rs` | `PendingSpawn::agent_prompt` | carry the prompt until a connecting pane lands |
| `src/ui/diff_overlay.rs`, `src/ui/document_column.rs` | `PendingSpawn` literals | `agent_prompt: None` |
| `src/ui/tab_sidebar.rs` | `tab_sidebar` group header | width budget for, and the child, `group_color::swatch` |
| `src/ui/tab_sidebar.rs` | `new_tab_in_group` | `new_tab_with_shell(None, ..)` so a group's New Tab skips the page |
| `src/ui/mod.rs` | module list | `agent_resume`, `group_color`, `new_tab_page` |
| `src/core/actions.rs` | actions list | `ContinueAllAgents`; `NewTabPageNextKind`, `NewTabPagePrevKind` (Tab on the new tab page) |
| `src/ui/keymap.rs` | `shipped_bindings`, `authored_entry`, `make_binding` | `ContinueAllAgents` |
| `src/ui/keymap.rs` | `fixed_bindings` | Tab / ⇧Tab bound in the `NewTabPage` context, since Root's focus walker otherwise takes Tab |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `ContinueAllAgents` in Search Everywhere |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `SearchText` (Search Text in Files…) |
| `src/ui/search/mod.rs` | `SearchTab`, `ORDER`, `title`, `placeholder`, module list (+ test) | the `Text` tab, on the row between Hosts and Actions |
| `src/ui/search/sources.rs` | `Catalog` fields, `new`, `source`, `all` | `text`, `text_query`; `all` leaves Text out |
| `src/ui/search/view.rs` | `perform_search`, `set_tab`, `render_empty`, `update_catalog` visibility, `text_rows` (test) | ask the window for text hits; the too-short hint |
| `src/ui/panel_search.rs` | module list | `pub(crate) mod model` for `split_relative` |
| `src/ui/app.rs` | `open_search` | `catalog.text_query = palette_text_query(..)` |
| `src/ui/app.rs` | `run_command` | dispatch `CommandKind::SearchText` |
| `src/ui/i18n/mod.rs` | `L10nKey` | `CmdContinueAllAgents*`, `NewTabPage*`, `CmdSearchText`, `SearchTabText`, `SearchPlaceholderText`, `SearchTextTooShort` |
| `src/ui/i18n/en.rs`, `zh.rs`, `ja.rs` | `translate_*` | those keys; `QuitStopServerBody` says tabs come back asleep |
| `docs/agents/sessions.mdx` | resume section | restore asleep, Continue All Agents, `--continue`, hook-free Claude resume |
| `docs/reference/configuration.mdx` | config table | the fork's config fields |
| `docs/window/sidebar.mdx` | Group colours | `group_colors` |
| `docs/window/search-everywhere.mdx` | Tabs table | the Text tab |
| `docs/docs.json` | "The window" pages | `window/new-tab-page` |
