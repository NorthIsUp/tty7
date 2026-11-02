# Settings → Agents edits each agent's model, permission choices and extra args

`Fork-Feature: agent-launch-settings`. The commit that carries this feature is `fork(agent-launch-settings): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/settings/agent_launch.rs` | an agent's launch options under its integration row (`render_agent_launch_panel`), a label column and a `kit::CONTROL_W` control column indented to the row's text, each CLI option with a trash tile (`icons/settings/trash.svg`, in `ui/assets.rs`): always for an agent with hooks installed, else opened from the ⋯ menu's Launch options… (`toggle_agent_launch`): Model, the agent's `launch_choices` dropdowns, Extra arguments rows (add/remove, each line checked with `split_shell_args`), the line it launches as; `launch_summary`, the collapsed row's note; open rows and their fields per window in `AgentLaunchUi`, every agent's built with the page (`build_agent_launch_inputs`) (+ tests: each agent's argv, defaults, quoting, resume/fork/relaunch, migration, summary; a gpui test that the grid lines up across agents and stays inside the row) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/cli_agent.rs` | `CLIAgent::model_flag`, `CLIAgent::launch_choices` (each option under the agent's own name and description: Claude Code's permission modes, Codex's `/permissions` presets from rust-v0.166.0's `builtin_approval_presets` and `permissions_menu.rs`, Cursor's modes), `LaunchChoice`, `LaunchOption` | the flags Settings → Agents maps its controls to, from each CLI's `--help` (claude 2.1.285, codex 0.166.0, pi 1.4.0, cursor-agent 2026.10.22) |
| `crates/tty7-core/src/core/fork_config.rs` | `ForkConfig::agent_settings`, `AgentLaunchSettings` (`flags`, `from_launch_line` — an option spanning several flags is taken when all are on the line —, `merge_codex_sandbox_and_approval`), `Config::launch_lines`, `Config::move_agent_launch_into_settings` | per-agent settings appended to `agent_launch`; a plain `agent_launch` line moves into them |
| `crates/tty7-core/src/core/config.rs` | `sanitize` | `move_agent_launch_into_settings` on every load |
| `src/ui/agent_launch.rs`, `src/ui/agent_resume.rs`, `src/ui/search/command.rs`, `src/ui/worktree_prompt.rs`, `src/ui/app.rs` (Agent command subtitle), `crates/tty7-cli/src/commands.rs` | `start_agent_in`, `resume_session`, `Resume::restored`'s fresh start, the session list's fork check, the worktree agent line | `Config::launch_lines` instead of `agent_launch`, so every launch carries the settings; a pane resumed into its own session keeps replaying its recorded flags |
| `src/ui/agent_launch.rs` | `resume_line`, `fork_line` (`split_shell_args`), `remember_launch_line` | a quoted extra splits as the shell would; Set Current Launch Args as Default writes the agent's settings when the line is plain, else `agent_launch`, and drops the settings either way |
| `src/ui/settings/kit.rs` | `raised_text_field` | a text field dressed as a `select_trigger`, for the launch options grid only |
| `src/ui/settings/shell.rs`, `src/ui/settings/agents.rs` | `MenuEntry::Item::detail`, drawn under the label by `settings_menu_panel`; `detail: None` in the other constructors | a choice's one-line description in its menu |
| `src/ui/settings.rs`, `src/ui/settings/agents.rs`, `src/ui/app.rs` | module list; `build_settings_state` (`build_agent_launch_inputs`); `render_agent_row` (the options under a hooked row; for the rest, Launch options… in the ⋯ menu, now shown without hooks too, and the summary under the hook path), `render_agent_hooks_group`'s filter | an agent with its Launch options open stays listed |
| `src/ui/i18n/{mod,en,zh,ja,ru}.rs` | after `SettingsShellStartupEnvlibs` | `SettingsAgentLaunchesAs`, `SettingsAgentModel`(`Desc`), `SettingsAgentPermission`, `SettingsAgentSandbox`, `SettingsAgentApproval`, `SettingsAgentChoiceDesc`, `SettingsAgentDefault`, `SettingsAgentExtraArgs`(`Desc`), `SettingsAgentAddArg`, `SettingsAgentRemoveArg`, `SettingsAgentLaunchOptions`, `SettingsAgentSummaryModel`, `SettingsAgentSummaryExtras` (plural) |
| `docs/agents/overview.mdx`, `docs/reference/configuration.mdx` | Quick launch; the agents table | the per-agent settings, what each maps to, which launches use them |
