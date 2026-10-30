# pane shells can start without the user's rc files

`Fork-Feature: shell-startup`. The commit that carries this feature is `fork(shell-startup): …` on `main-niu`.

`shell_startup` is `user` (upstream's behaviour), `no_rcs` or `no_rcs_envlibs`.
In the no-rcs modes the daemon writes a startup script into the zsh or bash
injection dir's `startup/` and points `TTY7_SHELL_STARTUP` at it; the zsh
redirectors and the bash rcfile source that instead of the user's files, so
tty7's integration still loads after it. fish gets `--no-config` and the script
as an extra `-C`. Other shells are untouched. The script runs `path_helper` on
macOS, then, in `no_rcs_envlibs`, the env tools `detect` found before spawn:
mise alone, else asdf's shims, direnv, and a `.env` parser when direnv is
missing. In zsh, `mise activate` is read in the background (`DEFER_ZSH`: a
process substitution, a `zle -F` handler, and a `preexec` that waits for it if
a command comes first), so the first prompt does not wait on mise. User docs: `docs/reference/shell-integration.mdx#skipping-your-rc-files`.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/daemon/shell_startup.rs` | detection, the per-shell startup script, `apply` on an `Injection` |
| `src/ui/shell_startup.rs` | the Shell startup settings row |
| `crates/tty7-server/tests/shell_startup.rs` | a real zsh pane per mode |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/daemon/shell_integration.rs` | `zsh_redirectors` | source `$TTY7_SHELL_STARTUP/<file>` instead of the user's when set |
| `crates/tty7-core/src/daemon/shell_integration.rs` | `bash_rcfile` | `if` round the login chain: source `$TTY7_SHELL_STARTUP/.bashrc` instead |
| `crates/tty7-core/src/daemon/shell_integration.rs` | `setup`, after the per-shell match | `shell_startup::apply(&kind, …)`; `zsh_redirectors`/`bash_rcfile` are `pub(super)` for its tests |
| `crates/tty7-core/src/daemon/mod.rs` | module list | `shell_startup` |
| `crates/tty7-core/src/core/fork_config.rs` | `ForkConfig` | `shell_startup`, `ShellStartup` |
| `src/ui/mod.rs` | module list | `shell_startup` |
| `src/ui/settings/pages.rs` | `render_shell_group`, after Arguments | `shell_startup_row` |
| `src/ui/i18n/{mod,en,ja,ru,zh}.rs` | after `GitHubOpenInGraphite` | `SettingsShellStartup*` |
| `docs/reference/{shell-integration,configuration}.mdx` | | the setting |
