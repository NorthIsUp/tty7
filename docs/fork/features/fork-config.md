# ForkConfig and the config the fork's features read

`Fork-Feature: fork-config`. The commit that carries this feature is `fork(fork-config): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/core/fork_config.rs` | `ForkConfig`: the fork's settings (resume, new tab page, group decoration, `nice`, GitHub panel, global hotkey), flattened into `Config`; `AgentResumeMode`, `GroupColorSource`, `GroupBackgroundScope`. The retired `github_panel_default_list` / `github_panel_session_filter` keys still load (ignored) |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/config.rs` | `Config::fork` (`#[serde(flatten)]`), `Default` | every fork setting lives in `fork_config::ForkConfig`, at the top level of `config.json` as before |
| `crates/tty7-core/src/core/mod.rs` | module list | `pub mod history_search`, `pub mod claude_background`, `pub mod fork_config`, `pub mod fork_host`, `pub mod history_cache` |
| `src/ui/i18n/mod.rs` | `L10nKey`, one block at the end of `l10n_keys!` under a `// fork` comment | `CmdContinueAllAgents*`, `NewTabPage*`, `SettingsGroup*`, `CmdSearchText`, `SearchTabText`, `SearchPlaceholderText`, `SearchTextTooShort`, `SearchTabHistory`, `SearchPlaceholderHistory`, `SearchHistory*`, `CmdSearchAgents`, `SearchTabAgents`, `SearchPlaceholderAgents`, `GitHubSession`, `GitHubAll`, `GitHubNoSessionMentions`, `GitHubNoSessionMatches`, `SearchHintBackground`, `SettingsHotkey*`, `Switcher{Set,Unset}HotkeyWorkspace`, `SwitcherHotkeyWorkspace` |
| `src/ui/i18n/en.rs`, `zh.rs`, `ja.rs` | `translate_*` | those keys, in one block at the end of each `match` under a `// fork` comment; `QuitStopServerBodyAsleep`, Quit and Stop's body when agents don't resume on launch (tabs come back asleep) |
| `docs/reference/configuration.mdx` | config table | the fork's config fields |
