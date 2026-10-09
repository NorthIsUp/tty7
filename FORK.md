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
- [fork-infra](docs/fork/features/fork-infra.md): the fork's tasks, packaging, CI and sync
- [fork-config](docs/fork/features/fork-config.md): ForkConfig and the config the fork's features read
- [fork-docs](docs/fork/features/fork-docs.md): the fork's specs, plans and feature pages
- [agent-resume](docs/fork/features/agent-resume.md): agents come back after a reboot
- [palette-tabs](docs/fork/features/palette-tabs.md): the New Tab picker and Search Everywhere's tabs
- [background-tabs](docs/fork/features/background-tabs.md): ⇧ opens a tab in the background
- [sidebar-groups](docs/fork/features/sidebar-groups.md): group colours, headers, fill and fold
- [pane-nice](docs/fork/features/pane-nice.md): pane shells start at the configured nice
- [github-session](docs/fork/features/github-session.md): the GitHub panel's Session tab and This session filter
- [github-review](docs/fork/features/github-review.md): the GitHub panel's pull request review box
- [github-merge](docs/fork/features/github-merge.md): the GitHub panel's pull request merge buttons and auto-merge
- [github-row-links](docs/fork/features/github-row-links.md): the GitHub panel's row links and pull request stacks
- [github-recent-branches](docs/fork/features/github-recent-branches.md): the GitHub panel keeps the last five branches
- [traffic-light-inset](docs/fork/features/traffic-light-inset.md): the traffic lights sit as far in from the left as from the top
- [handoff-modes](docs/fork/features/handoff-modes.md): a pane keeps its private modes across a daemon handoff
- [hotkey-window](docs/fork/features/hotkey-window.md): a global hotkey window
- [detached-replies](docs/fork/features/detached-replies.md): a pane no window shows still answers DA1
## Syncing
`mise run sync-upstream` folds every commit on `main-niu` into its feature's
commit (from the `Fork-Feature:` trailer a PR carries), rebases the result onto
`upstream/main`, runs the tests, and pushes with a lease. Upstream's changes to
a feature's hook show up as conflicts in that feature's commit only. See
`docs/fork/patch-stack.md`.
