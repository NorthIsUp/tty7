# Fork spec: main-niu as a patch stack

`main-niu` stops being a log of squashed PRs and becomes a curated stack: one
commit per fork feature, rebased onto upstream `main` on every sync. Two syncs
on 2026-09-30 replayed 52 and then 74 commits. The first stopped on 15
conflicts and the second on 5, and most of those came from fork history
fighting itself, not from upstream. A stack of about a dozen commits replays
each feature once, in its final shape.

## Goals

- A sync replays one commit per feature, so a conflict shows up once per
  feature that touches the changed upstream code, not once per historical
  commit that touched it.
- Ordinary work is unchanged: PRs still squash-merge into `main-niu` and CI
  gates them.
- Upstream files carry as few fork lines as possible, and the places upstream
  keeps editing carry none.
- Every rewrite of `main-niu` is checked mechanically before it is pushed.

Out of scope: upstreaming more features (that is separate work, feature by
feature), and changing what any feature does.

## The stack

Each stack commit is titled `fork(<slug>): <summary>` and carries a
`Fork-Feature: <slug>` trailer. The order, base first:

| slug | holds |
|---|---|
| `fork-infra` | mise tasks, the fast profile, tty7-niu packaging, `install-app` / `launch` / `reload`, `fork_update`, `sync-upstream`, `fold-plan`, `tag-on-bump.yml`, `niu-ci.yml`, the `release.yml` and `bundle-macos.sh` hooks |
| `fork-config` | `ForkConfig` and the config contracts the later features read |
| `fork-docs` | `docs/fork/**` and the FORK.md index |
| `agent-resume` | hook-free Claude resume, restore asleep, Continue All Agents, `--continue`, background attach, `ResumePlan`, the Settings switch |
| `new-tab-page` | the New Tab picker and the palette's New Tab tab |
| `background-tabs` | ⇧Enter / ⇧-click opens a tab in the background |
| `search-tabs` | Search Everywhere's Text, History and Agents tabs, `LiveTab` |
| `sidebar-groups` | group colours, headers, fill and fold, golden-angle hues |
| `pane-nice` | `Config::nice` and its rechecks |
| `github-session` | the GitHub panel's This session filter, fork default and Session tab |
| `hotkey-window` | the global hotkey window and its fixes |

Features upstream merged on 2026-10-01 (#1059–#1064: process CPU and memory,
DEC 2031, OSC 99 and program notifications, Move to Group, the sidebar scroll
fix, `.itermcolors` integers) dropped out when the stack was first built, and
have no stack commits. The fork's program-notes feature went with them: upstream's
version covers it, and its one remaining read (`laid_out_grid`) is
`background-tabs`'.

On 2026-10-03 upstream merged osc8-underline (#1078) and panel-tabs (#1079),
which left the stack, and took `bundle-macos.sh`'s overrides (#1067), which
left fork-infra.

A hunk shared by several features goes in the earliest feature in the order
above that uses it. The cross-cutting refactors (#43, #49, #50, #65) are split
by file and hunk among the features that own them.

### FORK.md

FORK.md becomes an index: what the fork is, the branch layout, how to sync,
and one line per feature linking to `docs/fork/features/<slug>.md`. Each
feature's page holds that feature's fork-owned files and its hook table, so
only that feature's commit ever edits it. Without this, every stack commit
edits FORK.md, and reordering or folding them conflicts on it each time.

## New work

A PR names its feature with a `Fork-Feature: <slug>` line in its body. The
repo's squash settings use the PR title and body as the commit message
(`squash_merge_commit_title=PR_TITLE`, `squash_merge_commit_message=PR_BODY`),
so the trailer reaches the squash commit. A PR template carries the line. A
PR without the trailer stays its own commit at the end of the stack, the one
line in `git log` without a `fork(` prefix, until a later PR tagged with its
slug folds it in. Guessing a slug from the commit's scope would regroup every
untagged commit that predates the stack.

Between syncs, `main-niu` is the stack plus a tail of these squash commits.

## Sync

`mise run sync-upstream` runs three phases, and stops at the first failure.

1. **Fold.** `mise-tasks/fold-plan` reads the stack and the tail and writes a
   rebase todo: each stack commit in order, then a `fixup` for each tail
   commit whose `Fork-Feature` matches it, in tail order. Untagged tail
   commits are picked at the end. The rebase runs on the current base with
   `GIT_SEQUENCE_EDITOR` pointed at the plan, so nothing is interactive.
   Check: the folded tip's tree is identical to `main-niu`'s
   (`git diff --quiet main-niu HEAD`). Folding only reorders and merges
   commits, so any difference is a bad resolution, and the sync stops.
2. **Rebase** onto `upstream/main`, with `rerere.enabled` and
   `rerere.autoupdate`. A stack commit whose change upstream now contains
   comes out empty and is dropped (`--empty=drop`). One that upstream took in
   a reworded form conflicts, and is resolved by keeping upstream's side.
3. **Publish** with `git push --force-with-lease=main-niu:<sha fetched in phase 1>`.
   If the lease fails because a PR merged mid-sync, the task fetches, folds
   only the commits that are new since that sha (they sit on top of the tip
   it already built), and pushes again. After three failed leases it stops
   and asks for a quiet minute.

`gh repo sync` keeps `main` mirroring upstream, as it does today.

Conflicts in phase 1 are between fork commits. They are rare, because a fold
moves a fix next to the feature it fixes, and rerere replays any one already
resolved. rerere keeps those resolutions in the repo's `.git/rr-cache`, which
every worktree shares.

## Footprint in upstream files

| file | today | after |
|---|---|---|
| `.github/workflows/ci.yml` | 23 fork lines (`main-niu`, macOS only) | identical to upstream; disabled with `gh workflow disable ci.yml` |
| `.github/workflows/niu-ci.yml` | none | fork-owned. Keeps the job names `rustfmt`, `build & test (aarch64-apple-darwin)` and `host boundary`, so branch protection's required checks match without a settings change |
| `.github/workflows/release.yml` | 13 fork lines | unchanged. A fork copy would miss upstream's packaging fixes without saying so; this hook rarely conflicts |
| `.github/workflows/nightly.yml` | 3 fork lines, workflow disabled | identical to upstream |
| `.github/scripts/bundle-macos.sh` | 77 fork lines, woven through the script: app name, bundle id, binary and dist dirs, local keychain signing, App Store Connect API key notarization | sent upstream as one PR, since every change is an opt-in env override; the fork keeps the hook until it merges. A fork-only copy would be nearly the whole script and drift from upstream's |
| `src/ui/i18n/{mod,en,ja,zh}.rs` | fork keys mixed among upstream's, and a run of upstream keys moved | one contiguous block at the end of `L10nKey` and of each `translate_*`, under a `// fork` comment |
| `docs/**.mdx` tables | one row per fork feature | unchanged |

`check-host-boundary.sh` keeps its two-line allowlist hook.

## Building the stack once

On a fresh branch from `main-niu`, after the next sync:

1. Make the footprint changes above as ordinary commits, and split FORK.md into
   the index and the feature pages.
2. Tag every commit since the upstream base with its slug, from the table
   above (a mapping file `docs/fork/stack-map.tsv`: sha, then slug). Split
   the cross-cutting refactors by hunk first.
3. Run phase 1 of the sync with every commit treated as tail and an empty
   stack, so the fold builds one commit per slug in the table's order.
4. Check the tree against the branch's tip (`git diff --quiet`), run the full
   test suite, and push `main-niu` with a lease, as in phase 3.

The old history stays on origin as `backup/main-niu-pre-stack`.

## Failure modes

- **A fold conflict that's hard to resolve.** Abort the sync. `main-niu` is
  unchanged, since nothing is pushed before phase 3.
- **A PR with the wrong slug.** Its fix folds into the wrong feature. The tree
  is still correct and only the stack's grouping is off; the next PR moves
  the hunk.
- **Branches cut from the old tip.** They rebase onto the new `main-niu` with
  `git rebase --onto origin/main-niu <old tip>`. The sync prints the old tip.

## Testing

- `fold-plan` gets a self-check script: a throwaway repo with a three-commit
  stack and a tagged tail, folded, then asserted to have the expected subjects
  and the original tree.
- Every sync runs the folded-tree equality check (phase 1) and the full test
  suite before phase 3.
- The first stack build is checked against the tip it replaces, and the
  required CI checks must pass on it before any later sync runs.
