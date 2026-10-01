# the fork's tasks, packaging, CI and sync

`Fork-Feature: fork-infra`. The commit that carries this feature is `fork(fork-infra): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `mise.toml`, `mise-tasks/` | tool pin; build, test, release and `sync-upstream`; `build-fast`, `install-app` (the fast build as `~/Applications/tty7-niu-dev.app`, signed), `launch` (that bundle, clean env) and `reload` (restarts the server in place when its code changed, so panes survive, then replaces the window); `set-release-secrets` (human-run, release.yml's signing secrets) |
| `src/core/fork_update.rs` | the update feed's repo (`update_repo!`, NorthIsUp/tty7); a local install checks no feed and prompts to restart when `install-app` lands a new build |
| `.github/workflows/tag-on-bump.yml` | on a `main-niu` push that bumps the workspace version: tag `v<version>` and dispatch `release.yml` on it |
| `crates/tty7-core/src/daemon/dev_config_guard.rs`, `crates/tty7-server/tests/dev_config_guard.rs` | a dev build (under a cargo target dir) will not serve the default config dir unless `TTY7_DEV_USE_REAL_CONFIG=1` (which `mise run run` sets; read once, then dropped from the env): `enforce` exits a fresh start up front, `handoff_refusal` keeps a server's panes from a dev build that would refuse, `refusal` leaves an adopting server serving panes only (+ tests) |
| `crates/tty7-core/src/core/dev_build.rs` | `is_dev_build` / `running_dev_build`: a binary under a dir holding `CACHEDIR.TAG`, or under a `target` beside a `Cargo.toml` (as launched or canonicalized), is a build and never links the CLI onto PATH or writes agent hooks; offered upstream as `upstream/dev-build-no-install` |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-server/tests/cli.rs`, `remote_router.rs`, `machine_tree.rs`, `stdio_conformance.rs` | `server()` and each spawn's `--config-dir` / `TTY7_DEV_USE_REAL_CONFIG` | every spawned server runs on a scratch config dir: `--stdio` with nothing listening serves in-process and rewrote the user's own `machine.json` |
| `crates/tty7-core/src/daemon/server.rs`, `crates/tty7-core/src/daemon/handoff.rs`, `crates/tty7-core/src/daemon/mod.rs`, `crates/tty7-server/src/main.rs` | `run_daemon` (`opted_in` right after the disclaim re-exec, then `enforce`) and the `--stdio` serve (`enforce`), `hand_over` (`handoff_refusal`), `handoff::take_over` (the opt-in on the exec), `control_services` (`refusal`), `mod dev_config_guard` | the dev-build guard |
| `Cargo.toml` | `[profile.fast]` | the day-to-day build: deps at opt 3, the app crate at opt 1, no LTO |
| `src/core/update.rs` | `REPO`, `RELEASES_URL`, `NIGHTLY_RELEASE_URL` | `update_repo!()`, so checks and links read the fork's releases |
| `src/core/cli_install.rs` (`install_inner`), `src/core/aumid.rs` (`is_build_output`), `crates/tty7-core/src/core/agent_hooks.rs` (`install_hooks`, `uninstall_hooks`: `refuse_a_dev_build`; `refresh_hooks_at_launch`) | the build check | ORs `dev_build::is_dev_build` into upstream's `in_a_build_tree` / `has_build_layout`, refuses a build's hook write into the user's own home (a scratch home is allowed), and skips the launch refresh on a build, so a build never rewrites `/opt/homebrew/bin/tty7` or `~/.claude/settings.json` |
| `src/ui/settings/agents.rs`, `src/ui/i18n/{mod,en,zh,ja}.rs` | the hook Install button, the agent hooks group; `SettingsAgentHooksDevBuild` | on a build, the local Install button is disabled, Reinstall and Uninstall are hidden, and a note names the build's path |
| `src/core/update.rs` | `spawn_check`, `spawn_check_inner` | `fork_update::watch`; a local install skips the GitHub check |
| `src/core/mod.rs` | module list | `pub mod fork_update` |
| `src/bin/tty7-updater.rs` | `install_inner`, `extract_archive` → `unpacked_app` (+ test) | find the unpacked `.app` rather than name `tty7.app`, since the fork's is `tty7-niu.app` |
| `.github/scripts/bundle-macos.sh` | top, Info.plist, signing, notarization, after the sweep | `TTY7_APP_NAME`, `TTY7_BUNDLE_ID`, `TTY7_BIN_DIR`, `TTY7_DIST`, `TTY7_LOCAL_BUILD_ID`, `TTY7_BUNDLE_ONLY`; sign with a keychain identity when no cert is imported (no timestamp); notarize with an ASC API key (`ASC_KEY_P8`, `ASC_KEY_ID`, `ASC_ISSUER_ID`) |
| `.github/workflows/ci.yml` | `on.push.branches`, the three `save-if`s; `build` matrix, `server-musl` `if`, `server-macos` `if` | `main-niu`, not `main`; macOS only, Windows, Linux and musl commented out; server macOS builds skip PRs |
| `.github/workflows/nightly.yml` | `plan` job `if` | runs only on `l0ng-ai/tty7`, so a re-enabled workflow on the fork publishes nothing |
| `.github/workflows/release.yml` | `Bundle macOS DMG` env; `draft-release` last step | build `tty7-niu.app` (`com.northisup.tty7-niu`) with the ASC notarization key; publish the draft on NorthIsUp/tty7 |
