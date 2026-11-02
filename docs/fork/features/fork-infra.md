# the fork's tasks, packaging, CI, sync and updater

`Fork-Feature: fork-infra`. The commit that carries this feature is `fork(fork-infra): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `mise.toml`, `mise-tasks/` | tool pin; build, test, release and `sync-upstream`; `build-fast`, `install-app` (the fast build as `~/Applications/tty7-niu-dev.app`, signed), `launch` (that bundle, clean env) and `reload` (restarts the server in place when its code changed, so panes survive, then replaces the window); `set-release-secrets` (human-run, release.yml's signing secrets) |
| `src/core/fork_update.rs` | the update feed's repo (`update_repo!`, NorthIsUp/tty7); a local install checks no feed and prompts to restart when `install-app` lands a new build |
| `.github/workflows/tag-on-bump.yml` | on a `main-niu` push that bumps the workspace version: tag `v<version>` and dispatch `release.yml` on it |
| `.github/workflows/niu-ci.yml` | the fork's CI: upstream's ci.yml narrowed to macOS and `main-niu`, its macOS server jobs skipped on PRs; same job names as the required checks. Upstream's `ci.yml` and `nightly.yml` stay byte-identical and are disabled on the fork |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `Cargo.toml` | `[profile.fast]` | the day-to-day build: deps at opt 3, the app crate at opt 1, no LTO |
| `src/core/update.rs` | `REPO`, `RELEASES_URL`, `NIGHTLY_RELEASE_URL` | `update_repo!()`, so checks and links read the fork's releases |
| `src/core/update.rs` | `spawn_check`, `spawn_check_inner` | `fork_update::watch`; a local install skips the GitHub check |
| `src/core/mod.rs` | module list | `pub mod fork_update` |
| `src/bin/tty7-updater.rs` | `install_inner`, `extract_archive` → `unpacked_app` (+ test) | find the unpacked `.app` rather than name `tty7.app`, since the fork's is `tty7-niu.app` |
| `.github/workflows/release.yml` | `Bundle macOS DMG` env; `draft-release` last step | build `tty7-niu.app` (`com.northisup.tty7-niu`) with the ASC notarization key; publish the draft on NorthIsUp/tty7 |
