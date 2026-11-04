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

## Features

Each feature is one commit on `main-niu`, titled `fork(<slug>): …`. Its page
lists the files it owns and the hooks it adds to upstream files: the table to
read when a sync stops on a conflict in that feature.

- [fork-infra](docs/fork/features/fork-infra.md): the fork's tasks, packaging, CI, sync and updater
- [fork-docs](docs/fork/features/fork-docs.md): the fork's specs, plans and feature pages
- [fork-config](docs/fork/features/fork-config.md): the fork's settings section in config.json
- [dev-build-guard](docs/fork/features/dev-build-guard.md): a cargo build never writes the user's config dir or agent hooks
- [first-prompt](docs/fork/features/first-prompt.md): a typed launch waits for the shell's first prompt
- [run-once](docs/fork/features/run-once.md): a new pane runs one command at its first prompt without making it the pane's shell
- [spawn-grid](docs/fork/features/spawn-grid.md): a hidden pane spawns at the window's real grid, not 80x24
- [agent-launch-settings](docs/fork/features/agent-launch-settings.md): Settings → Agents edits each agent's model, permission choices and extra args
- [agent-resume](docs/fork/features/agent-resume.md): agents come back after a reboot, asleep, woken a few at a time
- [agent-startup-prompts](docs/fork/features/agent-startup-prompts.md): answers an agent's startup prompts and offers to restart a draining Codex server
- [search-text](docs/fork/features/search-text.md): Search Everywhere's Text tab finds text in files
- [pane-addresses](docs/fork/features/pane-addresses.md): ⌘P takes a pane or tab address the way the CLI prints it
- [search-history](docs/fork/features/search-history.md): Search Everywhere's History tab searches past agent sessions
- [open-sessions](docs/fork/features/open-sessions.md): the Sessions tab leads with agent sessions already open in a tab
- [palette-tabs](docs/fork/features/palette-tabs.md): New Tab asks what to open and where
- [background-tabs](docs/fork/features/background-tabs.md): ⇧ opens a tab in the background
- [panel-selected-contrast](docs/fork/features/panel-selected-contrast.md): the selected sidebar and panel row stands further off its background
- [sidebar-groups](docs/fork/features/sidebar-groups.md): group colours, outline and fill, animated fold, default branch in the header
- [group-follow](docs/fork/features/group-follow.md): a pinned folder group holds the tabs whose cwd is in it; a tab can be held
- [pane-nice](docs/fork/features/pane-nice.md): pane shells start at the configured nice
- [github-session](docs/fork/features/github-session.md): the GitHub panel's Session tab and This session filter
- [github-review](docs/fork/features/github-review.md): the GitHub panel's pull request review box
- [github-merge](docs/fork/features/github-merge.md): the GitHub panel's pull request merge buttons and auto-merge
- [hotkey-window](docs/fork/features/hotkey-window.md): a global hotkey window
- [github-row-links](docs/fork/features/github-row-links.md): the GitHub panel's Open in Graphite tile
- [github-recent-branches](docs/fork/features/github-recent-branches.md): the GitHub panel keeps the last five branches
- [tab-usage](docs/fork/features/tab-usage.md): the sidebar shows each tab's pid, CPU and memory
- [shell-startup](docs/fork/features/shell-startup.md): pane shells can start without the user's rc files
- [spawn-trace](docs/fork/features/spawn-trace.md): each new tab logs its start as one timeline
- [quiet-close](docs/fork/features/quiet-close.md): an untouched new tab closes without asking
- [fork-group-repair](docs/fork/features/fork-group-repair.md): one-shot repair of tabs an older fork build filed in the wrong folder group
- [fork-config-compat](docs/fork/features/fork-config-compat.md): a config.json from before ForkConfig loads and saves every fork key
- [codex-tool-cwd](docs/fork/features/codex-tool-cwd.md): a Codex command run in another directory leaves the tab where it is

## Syncing

`mise run sync-upstream` folds every commit on `main-niu` into its feature's
commit (from the `Fork-Feature:` trailer a PR carries), rebases the result onto
`upstream/main`, runs the tests, and pushes with a lease. Upstream's changes to
a feature's hook show up as conflicts in that feature's commit only. See
`docs/fork/patch-stack.md`.
