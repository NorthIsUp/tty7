# a cargo build never writes the user's config dir or agent hooks

`Fork-Feature: dev-build-guard`. The commit that carries this feature is `fork(dev-build-guard): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/daemon/dev_config_guard.rs`, `crates/tty7-server/tests/dev_config_guard.rs` | a dev build (under a cargo target dir) will not serve the default config dir unless `TTY7_DEV_USE_REAL_CONFIG=1` (which `mise run run` sets; read once, then dropped from the env): `enforce` exits a fresh start up front, `handoff_refusal` keeps a server's panes from a dev build that would refuse, `refusal` leaves an adopting server serving panes only (+ tests) |
| `crates/tty7-core/src/core/dev_build.rs` | `is_dev_build` / `running_dev_build`: a binary under a dir holding `CACHEDIR.TAG`, or under a `target` beside a `Cargo.toml` (as launched or canonicalized), is a build and never links the CLI onto PATH or writes agent hooks; offered upstream as `upstream/dev-build-no-install` |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-server/tests/cli.rs`, `remote_router.rs`, `machine_tree.rs`, `stdio_conformance.rs` | `server()` and each spawn's `--config-dir` / `TTY7_DEV_USE_REAL_CONFIG` | every spawned server runs on a scratch config dir: `--stdio` with nothing listening serves in-process and rewrote the user's own `machine.json` |
| `crates/tty7-core/src/daemon/server.rs`, `crates/tty7-core/src/daemon/handoff.rs`, `crates/tty7-core/src/daemon/mod.rs`, `crates/tty7-server/src/main.rs` | `run_daemon` (`opted_in` right after the disclaim re-exec, then `enforce` unless adopting a handoff, before stderr is captured so the refusal reaches the caller) and the `--stdio` serve (`enforce`), `hand_over` (`handoff_refusal`), `handoff::take_over` (the opt-in on the exec), `control_services` (`refusal`), `mod dev_config_guard` | the dev-build guard |
| `src/core/cli_install.rs` (`install_inner`), `src/core/aumid.rs` (`is_build_output`), `crates/tty7-core/src/core/agent_hooks.rs` (`install_hooks`, `uninstall_hooks`: `refuse_a_dev_build`; `refresh_hooks_at_launch`) | the build check | ORs `dev_build::is_dev_build` into upstream's `in_a_build_tree` / `has_build_layout`, refuses a build's hook write into the user's own home (a scratch home is allowed), and skips the launch refresh on a build, so a build never rewrites `/opt/homebrew/bin/tty7` or `~/.claude/settings.json` |
| `src/ui/settings/agents.rs`, `src/ui/i18n/{mod,en,zh,ja}.rs` | the hook Install button, the agent hooks group; `SettingsAgentHooksDevBuild` | on a build, the local Install button is disabled, Reinstall and Uninstall are hidden, and a note names the build's path |
