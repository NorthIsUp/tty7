# main-niu Patch Stack Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `main-niu` into one commit per fork feature, folded and rebased onto upstream by `mise run sync-upstream`, and cut the fork's lines in upstream files.

**Architecture:** `mise-tasks/fold-plan` (Python, stdlib only) reads the commits between upstream's base and `main-niu`, and prints a `git rebase -i` todo. The todo groups commits by feature slug (from a `fork(<slug>):` subject, a `Fork-Feature:` trailer, or a one-time map file) in `docs/fork/stack-order.tsv` order. `mise-tasks/sync-upstream` (bash) runs that rebase non-interactively, checks the tree didn't change, rebases onto `upstream/main`, runs the tests, and pushes with a lease, refolding and retrying if `main-niu` moved. Footprint work (a fork-owned CI workflow, per-feature FORK pages, one i18n block, an upstream PR for the bundle script) lands as ordinary squash PRs first. The last task builds the stack.

**Tech Stack:** git ≥ 2.38 (`rebase --empty=drop`, `%(trailers)`), bash, Python 3 stdlib, mise file tasks, GitHub Actions, `gh`.

**Spec:** `docs/fork/patch-stack.md`

## Global Constraints

- `main-niu` is the default branch. `main` mirrors upstream l0ng-ai/tty7 and never gets fork commits.
- PRs land by squash only. `main-niu` requires linear history and allows force-push only with `--force-with-lease`.
- Stack commit title: `fork(<slug>): <summary>`. Trailer: `Fork-Feature: <slug>`. Slugs match `[a-z0-9-]+`.
- Required checks on `main-niu`, names exact: `rustfmt`, `build & test (aarch64-apple-darwin)`, `host boundary`.
- Never `git commit --no-verify` or `git push --no-verify`. Never use bare `git stash`; the stack is shared across worktrees.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. PR bodies end with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- Cargo runs as `mise x rust@stable -- cargo …`.
- mise file tasks are executable scripts in `mise-tasks/` with a `#MISE description="…"` line.
- Repo settings, workflow enabling or disabling, and force-pushes to `main-niu` are human-run steps (the agent's permission check blocks them). Print the exact command and wait.

## Review Focus

1. **A PR merges into `main-niu` mid-sync.** The lease fails; sync fetches, refolds the new commits and retries, up to three times. Pinned in Task 2 (`race` case).
2. **A tail commit whose `Fork-Feature` slug isn't in `stack-order.tsv`** (a typo, or a new feature). It becomes its own `fork(<slug>):` group after the known ones, never dropped. Pinned in Task 1 (`unknown slug` case).
3. **Sync run with uncommitted changes.** It refuses before touching anything. Pinned in Task 2 (`dirty` case).
4. **A merge commit in the fork range.** `fold-plan` refuses with the merge's sha. Pinned in Task 1 (`merge` case).
5. **A stack summary containing quotes or `$`.** The amend command stays quoted, and the title comes out exactly. Pinned in Task 1 (`quote` case).

---

### Task 1: `fold-plan`

**Files:**
- Create: `mise-tasks/fold-plan`
- Create: `mise-tasks/fold-plan-check`

**Interfaces:**
- Produces: `mise-tasks/fold-plan BASE TIP ORDER_TSV [MAP_TSV]` prints a rebase todo on stdout and exits non-zero on a merge commit. `ORDER_TSV` lines are `slug<TAB>summary`. `MAP_TSV` lines are `sha<TAB>slug`, with full or abbreviated shas.
- Produces: `mise-tasks/fold-plan-check`, which exits 0 when every case passes.

- [ ] **Step 1: Write the failing check**

`mise-tasks/fold-plan-check`:

```bash
#!/usr/bin/env bash
#MISE description="Self-check for fold-plan: folds throwaway repos and asserts subjects and trees"
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
fail() { echo "FAIL: $*" >&2; exit 1; }

repo() { # fresh repo in a temp dir, cwd moves into it
  cd "$(mktemp -d)"
  git init -q -b main
  git config user.email t@t; git config user.name t; git config commit.gpgsign false
  echo base > base.txt; git add -A; git commit -qm base
}
commit() { # commit FILE CONTENT MESSAGE...
  local f=$1 c=$2; shift 2
  printf '%s\n' "$c" >> "$f"; git add -A
  local args=(); for m in "$@"; do args+=(-m "$m"); done
  git commit -q "${args[@]}"
}
fold() { # fold ORDER [MAP]: rebase base..HEAD with fold-plan's todo
  local plan; plan=$(mktemp)
  "$here/fold-plan" "$(git rev-list --max-parents=0 HEAD)" HEAD "$@" > "$plan"
  git -c sequence.editor="cp $plan" rebase -q -i "$(git rev-list --max-parents=0 HEAD)"
}
subjects() { git log --reverse --format=%s "$(git rev-list --max-parents=0 HEAD)..HEAD"; }

# sync mode: stack commits, a tagged tail, an unknown slug, an untagged commit
repo
printf 'a\tA feature\nb\tB feature\n' > order.tsv; order=$PWD/order.tsv
commit a.txt 1 "fork(a): A feature" "Fork-Feature: a"
commit b.txt 1 "fork(b): B feature" "Fork-Feature: b"
commit b.txt 2 "fix(b): tweak" "Fork-Feature: b"
commit a.txt 2 "fix(a): tweak" "Fork-Feature: a"
commit c.txt 1 "feat(c): new thing" "Fork-Feature: c"
commit u.txt 1 "chore: untagged"
before=$(git rev-parse HEAD^{tree})
fold "$order"
[ "$(git rev-parse HEAD^{tree})" = "$before" ] || fail "sync mode changed the tree"
want=$'fork(a): A feature\nfork(b): B feature\nfork(c): new thing\nchore: untagged'
[ "$(subjects)" = "$want" ] || fail "sync mode subjects: $(subjects)"
[ "$(git log -1 --format='%(trailers:key=Fork-Feature,valueonly)' HEAD~1 | tr -d '\n')" = c ] \
  || fail "unknown slug lost its trailer"

# build mode: no stack commits, slugs from a map, a summary with quotes and $
repo
printf "x\tdon't \$break \"it\"\n" > order.tsv; order=$PWD/order.tsv
commit x.txt 1 "feat: one"
commit y.txt 1 "feat: two"
commit x.txt 2 "fix: three"
git log --reverse --format='%H' HEAD~3..HEAD | sed -n '1p;3p' | sed 's/$/\tx/' > map.tsv
before=$(git rev-parse HEAD^{tree})
fold "$order" "$PWD/map.tsv"
[ "$(git rev-parse HEAD^{tree})" = "$before" ] || fail "build mode changed the tree"
want=$'fork(x): don\'t $break "it"\nfeat: two'
[ "$(subjects)" = "$want" ] || fail "build mode subjects: $(subjects)"

# merge: refused
repo
printf 'a\tA\n' > order.tsv
git switch -qc side; commit s.txt 1 "side"; git switch -q main; commit m.txt 1 "main"
git merge -q --no-edit side
if "$here/fold-plan" "$(git rev-list --max-parents=0 HEAD)" HEAD order.tsv >/dev/null 2>&1; then
  fail "fold-plan accepted a merge commit"
fi

echo "fold-plan-check: ok"
```

Make it executable: `chmod +x mise-tasks/fold-plan-check`.

- [ ] **Step 2: Run it and see it fail**

Run: `mise run fold-plan-check`
Expected: FAIL. `mise-tasks/fold-plan` does not exist, so the shell reports `fold-plan: No such file or directory`.

- [ ] **Step 3: Write `fold-plan`**

`mise-tasks/fold-plan`:

```python
#!/usr/bin/env python3
#MISE description="Print the rebase todo that folds main-niu's commits into one per feature (docs/fork/patch-stack.md)"
"""fold-plan BASE TIP ORDER_TSV [MAP_TSV]

Groups BASE..TIP by feature slug and prints a `git rebase -i BASE` todo: each
group's first commit picked, the rest fixed up into it, then retitled
`fork(<slug>): <summary>` if it isn't already. A commit's slug comes from
MAP_TSV, else a `fork(<slug>):` subject, else its `Fork-Feature` trailer.
Groups follow ORDER_TSV; slugs it doesn't list follow in first-seen order;
untagged commits are picked last, in order, untouched.
"""
import re
import shlex
import subprocess
import sys

STACK_SUBJECT = re.compile(r"^fork\(([a-z0-9-]+)\): ")
CONVENTIONAL = re.compile(r"^[a-z]+(\([^)]*\))?!?: ")


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def read_tsv(path: str) -> list[tuple[str, str]]:
    rows = []
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            if line and not line.startswith("#"):
                key, _, value = line.partition("\t")
                rows.append((key.strip(), value.strip()))
    return rows


def main(base: str, tip: str, order_path: str, map_path: str | None) -> None:
    merges = git("rev-list", "--merges", f"{base}..{tip}").split()
    if merges:
        sys.exit(f"fold-plan: {merges[0][:8]} is a merge commit; the stack is linear")
    order = read_tsv(order_path)
    summaries = dict(order)
    mapping = read_tsv(map_path) if map_path else []

    def mapped(sha: str) -> str | None:
        return next((slug for key, slug in mapping if sha.startswith(key)), None)

    groups: dict[str, list[tuple[str, str]]] = {}
    untagged: list[str] = []
    for sha in git("rev-list", "--reverse", f"{base}..{tip}").split():
        subject = git("log", "-1", "--format=%s", sha).rstrip("\n")
        trailer = git("log", "-1", "--format=%(trailers:key=Fork-Feature,valueonly,separator=%x2C)", sha)
        stack = STACK_SUBJECT.match(subject)
        slug = mapped(sha) or (stack and stack.group(1)) or trailer.split(",")[0].strip() or None
        if slug:
            groups.setdefault(slug, []).append((sha, subject))
        else:
            untagged.append(sha)

    known = [slug for slug, _ in order if slug in groups]
    extra = [slug for slug in groups if slug not in summaries]
    for slug in known + extra:
        (first, subject), *rest = groups[slug]
        print(f"pick {first}")
        for sha, _ in rest:
            print(f"fixup {sha}")
        summary = summaries.get(slug) or CONVENTIONAL.sub("", subject)
        title = f"fork({slug}): {summary}"
        if subject != title or rest:
            msg = shlex.quote(title)
            trailer = shlex.quote(f"Fork-Feature: {slug}")
            print(f"exec git commit --amend --quiet -m {msg} -m {trailer}")
    for sha in untagged:
        print(f"pick {sha}")


if __name__ == "__main__":
    if len(sys.argv) not in (4, 5):
        sys.exit(__doc__)
    main(*sys.argv[1:4], sys.argv[4] if len(sys.argv) == 5 else None)
```

Make it executable: `chmod +x mise-tasks/fold-plan`.

- [ ] **Step 4: Run the check and see it pass**

Run: `mise run fold-plan-check`
Expected: `fold-plan-check: ok`

- [ ] **Step 5: Commit**

```bash
git add mise-tasks/fold-plan mise-tasks/fold-plan-check
git commit -m "feat(fork): fold-plan groups main-niu's commits into one per feature" \
  -m "Fork-Feature: fork-infra" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `sync-upstream` folds, rebases and pushes with a lease

**Files:**
- Modify: `mise-tasks/sync-upstream` (whole file)
- Create: `mise-tasks/sync-upstream-check`

**Interfaces:**
- Consumes: `mise-tasks/fold-plan BASE TIP ORDER_TSV [MAP_TSV]` (Task 1), resolved next to this script.
- Produces: `mise run sync-upstream`, which reads `docs/fork/stack-order.tsv` from the repo being synced. Env overrides, for the check and for the one-time build: `SYNC_ORIGIN` (default `origin`), `SYNC_UPSTREAM` (`upstream`), `SYNC_BRANCH` (`main-niu`), `SYNC_FROM` (the ref to start from; default `<origin>/<branch>`, whose sha is always the lease), `SYNC_MAP` (a `MAP_TSV`, unset by default), `SYNC_SKIP_MIRROR` (skip `gh repo sync`), `SYNC_SKIP_TESTS` (skip `mise run test`), `SYNC_SKIP_PUSH` (stop after the tests and print the push command), `SYNC_BEFORE_PUSH` (a command run before each push attempt; test seam).

- [ ] **Step 1: Write the failing check**

`mise-tasks/sync-upstream-check`:

```bash
#!/usr/bin/env bash
#MISE description="Self-check for sync-upstream against throwaway origin and upstream repos"
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
fail() { echo "FAIL: $*" >&2; exit 1; }
export SYNC_SKIP_MIRROR=1 SYNC_SKIP_TESTS=1 GIT_CONFIG_NOSYSTEM=1
ident() { git config user.email t@t; git config user.name t; git config commit.gpgsign false; }

setup() { # $root/{upstream,origin}.git, a work clone at $root/work
  root=$(mktemp -d)
  git init -q --bare -b main "$root/upstream.git"
  git clone -q "$root/upstream.git" "$root/seed"; (cd "$root/seed" && ident \
    && echo base > base.txt && git add -A && git commit -qm base && git push -q origin main)
  git clone -q --bare "$root/upstream.git" "$root/origin.git"
  git clone -q "$root/origin.git" "$root/work"; cd "$root/work"; ident
  git remote add upstream "$root/upstream.git"
  git switch -qc main-niu
  mkdir -p docs/fork; printf 'a\tA feature\n' > docs/fork/stack-order.tsv
  echo a > a.txt; git add -A; git commit -qm "fork(a): A feature" -m "Fork-Feature: a"
  echo a2 >> a.txt; git commit -qam "fix(a): tweak" -m "Fork-Feature: a"
  git push -q origin main-niu
  (cd "$root/seed" && echo u > u.txt && git add -A && git commit -qm "upstream: u" && git push -q origin main)
}
tip_subjects() { git -C "$root/origin.git" log --format=%s main-niu; }

# happy path: tail folded, rebased onto upstream, pushed
setup
"$here/sync-upstream" >/dev/null
[ "$(tip_subjects)" = $'fork(a): A feature\nupstream: u\nbase' ] || fail "happy: $(tip_subjects)"

# a stack commit upstream already has is dropped
setup
(cd "$root/seed" && git fetch -q "$root/origin.git" main-niu && git cherry-pick FETCH_HEAD~1 FETCH_HEAD >/dev/null \
  && git reset -q --soft HEAD~2 && git commit -qm "upstream: took a" && git push -q origin main)
"$here/sync-upstream" >/dev/null
[ "$(tip_subjects | head -1)" = "upstream: took a" ] || fail "drop: $(tip_subjects)"

# a PR merges mid-sync: the lease fails once, the new commit is refolded
setup
race="$root/race.sh"; cat > "$race" <<EOF
[ -e "$root/raced" ] && exit 0; touch "$root/raced"
git clone -q "$root/origin.git" "$root/pr" -b main-niu; cd "$root/pr"
git config user.email t@t; git config user.name t; git config commit.gpgsign false
echo pr >> a.txt; git commit -qam "fix(a): from a PR" -m "Fork-Feature: a"; git push -q origin main-niu
EOF
SYNC_BEFORE_PUSH="bash $race" "$here/sync-upstream" >/dev/null
[ "$(tip_subjects)" = $'fork(a): A feature\nupstream: u\nbase' ] || fail "race: $(tip_subjects)"
git -C "$root/origin.git" show main-niu:a.txt | grep -qx pr || fail "race: the PR's change is missing"

# a dirty worktree is refused before anything changes
setup
echo dirt >> a.txt
before=$(git -C "$root/origin.git" rev-parse main-niu)
if "$here/sync-upstream" >/dev/null 2>&1; then fail "dirty: sync ran"; fi
[ "$(git -C "$root/origin.git" rev-parse main-niu)" = "$before" ] || fail "dirty: origin changed"

# SYNC_FROM: a local rewrite with the same tree starts the sync; origin is still the lease
setup
git switch -qc local; git reset -q --soft HEAD~2; git commit -qm "untagged rewrite" -m "Fork-Feature: a"
SYNC_FROM=local "$here/sync-upstream" >/dev/null
[ "$(tip_subjects)" = $'fork(a): A feature\nupstream: u\nbase' ] || fail "from: $(tip_subjects)"

echo "sync-upstream-check: ok"
```

`chmod +x mise-tasks/sync-upstream-check`

- [ ] **Step 2: Run it and see it fail**

Run: `mise run sync-upstream-check`
Expected: FAIL on the happy path. Today's `sync-upstream` rebases without folding, so `tip_subjects` still shows `fix(a): tweak`.

- [ ] **Step 3: Rewrite `sync-upstream`**

`mise-tasks/sync-upstream`:

```bash
#!/usr/bin/env bash
#MISE description="Fold main-niu into one commit per feature, rebase it onto upstream, test, and push with a lease (docs/fork/patch-stack.md)"
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
origin=${SYNC_ORIGIN:-origin} upstream=${SYNC_UPSTREAM:-upstream} branch=${SYNC_BRANCH:-main-niu}
rr=(-c rerere.enabled=true -c rerere.autoupdate=true)

git diff --quiet && git diff --cached --quiet \
  || { echo "sync-upstream: commit or set aside your changes first" >&2; exit 1; }
git remote get-url "$upstream" >/dev/null 2>&1 || git remote add "$upstream" https://github.com/l0ng-ai/tty7.git
git fetch -q "$upstream" main
git fetch -q "$origin" "$branch"
[ -n "${SYNC_SKIP_MIRROR:-}" ] || gh repo sync NorthIsUp/tty7 --branch main --source l0ng-ai/tty7

# Fold BASE..HEAD into one commit per feature. Folding only regroups commits,
# so a tree that differs afterwards means a conflict was resolved wrongly.
fold() {
  local base=$1 before plan
  before=$(git rev-parse HEAD)
  plan=$(mktemp)
  "$here/fold-plan" "$base" HEAD docs/fork/stack-order.tsv ${SYNC_MAP:+"$SYNC_MAP"} > "$plan"
  git "${rr[@]}" -c sequence.editor="cp $plan" rebase -q -i "$base" \
    || { echo "sync-upstream: fold stopped on a conflict; resolve, git rebase --continue, rerun" >&2; exit 1; }
  git diff --quiet "$before" HEAD \
    || { echo "sync-upstream: the fold changed the tree (compare $before with HEAD)" >&2; exit 1; }
}

lease=$(git rev-parse "$origin/$branch")
git switch -q --detach "${SYNC_FROM:-$lease}"
fold "$(git merge-base HEAD "$upstream/main")"
git "${rr[@]}" rebase -q --empty=drop "$upstream/main" \
  || { echo "sync-upstream: rebase stopped; keep upstream's side, re-add the hook (docs/fork/features/), git rebase --continue, rerun" >&2; exit 1; }
[ -n "${SYNC_SKIP_TESTS:-}" ] || mise run test

if [ -n "${SYNC_SKIP_PUSH:-}" ]; then
  echo "ready: git push --force-with-lease=$branch:$lease $origin HEAD:$branch"
  exit 0
fi
for _ in 1 2 3; do
  [ -z "${SYNC_BEFORE_PUSH:-}" ] || eval "$SYNC_BEFORE_PUSH"
  if git push -q --force-with-lease="$branch:$lease" "$origin" "HEAD:$branch"; then
    echo "pushed $branch. Branches cut from the old tip: git rebase --onto $origin/$branch $lease"
    exit 0
  fi
  git fetch -q "$origin" "$branch"
  new=$(git rev-parse "$origin/$branch")
  git merge-base --is-ancestor "$lease" "$new" \
    || { echo "sync-upstream: $branch was rewritten by someone else; rerun" >&2; exit 1; }
  git "${rr[@]}" cherry-pick "$lease..$new" \
    || { echo "sync-upstream: a commit merged mid-sync conflicts; resolve, git cherry-pick --continue, rerun" >&2; exit 1; }
  fold "$(git merge-base HEAD "$upstream/main")"
  lease=$new
done
echo "sync-upstream: $branch kept moving; wait for a quiet minute and rerun" >&2
exit 1
```

- [ ] **Step 4: Run the check and see it pass**

Run: `mise run sync-upstream-check && mise run fold-plan-check`
Expected: `sync-upstream-check: ok` then `fold-plan-check: ok`

- [ ] **Step 5: Add `stack-order.tsv`, so the real repo can run the new sync**

`docs/fork/stack-order.tsv` (slug, TAB, summary; the order is the stack's, base first):

```
fork-infra	the fork's tasks, packaging, CI and sync
fork-config	ForkConfig and the config the fork's features read
fork-docs	the fork's specs, plans and feature pages
agent-resume	agents come back after a reboot
new-tab-page	New Tab opens a picker for what to open and where
background-tabs	⇧ opens a tab in the background
search-tabs	Search Everywhere's Text, History and Agents tabs
sidebar-groups	group colours, headers, fill and fold
pane-nice	pane shells start at the configured nice
github-session	the GitHub panel's Session tab and This session filter
osc8-underline	OSC 8 links rest under a faint dotted underline
hotkey-window	a global hotkey window
program-notes	program notifications follow the per-pane rule
```

- [ ] **Step 6: Dry-run on the real repo, without pushing**

Run, in a clean worktree off `origin/main-niu`: `SYNC_SKIP_PUSH=1 SYNC_SKIP_MIRROR=1 mise run sync-upstream`
Expected: it ends `ready: git push --force-with-lease=main-niu:<sha> origin HEAD:main-niu`. With no stack commits and no trailers yet, every commit is untagged, so the fold keeps history as it is and the run is an ordinary rebase. Don't push. `git switch -` to leave the detached HEAD.

- [ ] **Step 7: Commit and open the PR**

```bash
git add mise-tasks/sync-upstream mise-tasks/sync-upstream-check docs/fork/stack-order.tsv
git commit -m "feat(fork): sync-upstream folds the stack, rebases, tests and pushes with a lease" \
  -m "Fork-Feature: fork-infra" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Open one PR against `main-niu` for Tasks 1 and 2, with `Fork-Feature: fork-infra` in its body.

---

### Task 3: Per-feature FORK pages

**Files:**
- Create: `docs/fork/features/<slug>.md`, one per slug in `docs/fork/stack-order.tsv`
- Modify: `FORK.md` (becomes an index)

**Interfaces:**
- Consumes: `docs/fork/stack-order.tsv` (Task 2).
- Produces: `docs/fork/features/<slug>.md`, each with two sections: `## Fork-owned files` (a `| file | what it holds |` table) and `## Hooks in upstream files` (a `| file | function / site | why |` table). Task 8 and later syncs point conflict resolution at these.

- [ ] **Step 1: Write the failing check**

Append to `mise-tasks/fold-plan-check`, before its final `echo`:

```bash
# every slug has a feature page, and FORK.md links each one
top=$(git -C "$here" rev-parse --show-toplevel)
while IFS=$'\t' read -r slug _; do
  [ -f "$top/docs/fork/features/$slug.md" ] || fail "no feature page for $slug"
  grep -q "docs/fork/features/$slug.md" "$top/FORK.md" || fail "FORK.md doesn't link $slug"
done < "$top/docs/fork/stack-order.tsv"
```

- [ ] **Step 2: Run it and see it fail**

Run: `mise run fold-plan-check`
Expected: `FAIL: no feature page for fork-infra`

- [ ] **Step 3: Split FORK.md**

For each row of FORK.md's two tables, decide its slug from what the row's file holds, using the spec's stack table (`docs/fork/patch-stack.md`). Move the row into that slug's page, keeping its exact text. A row naming several features goes to the earliest slug in `stack-order.tsv` that uses it. Each page starts:

```markdown
# <summary from stack-order.tsv>

`Fork-Feature: <slug>`. The commit that carries this feature is `fork(<slug>): …` on `main-niu`.
```

Then rewrite FORK.md as the index. Keep its intro paragraph and its "The app" section unchanged. Replace both tables with:

```markdown
## Features

Each feature is one commit on `main-niu`, titled `fork(<slug>): …`. Its page
lists the files it owns and the hooks it adds to upstream files: the table to
read when a sync stops on a conflict in that feature.

- [fork-infra](docs/fork/features/fork-infra.md): the fork's tasks, packaging, CI and sync
```

with one line per slug, in `stack-order.tsv` order and with its summary. Add a `## Syncing` section with exactly this text:

```markdown
## Syncing

`mise run sync-upstream` folds every commit on `main-niu` into its feature's
commit (from the `Fork-Feature:` trailer a PR carries), rebases the result onto
`upstream/main`, runs the tests, and pushes with a lease. Upstream's changes to
a feature's hook show up as conflicts in that feature's commit only. See
`docs/fork/patch-stack.md`.
```

Check that no row was lost:

Run: `git show HEAD:FORK.md | grep -c '^| \`'` and `cat docs/fork/features/*.md | grep -c '^| \`'`
Expected: the same number.

- [ ] **Step 4: Run the check and see it pass**

Run: `mise run fold-plan-check`
Expected: `fold-plan-check: ok`

- [ ] **Step 5: Commit and open the PR**

```bash
git add FORK.md docs/fork/features mise-tasks/fold-plan-check
git commit -m "docs(fork): one FORK page per feature; FORK.md is the index" \
  -m "Fork-Feature: fork-docs" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

The PR body carries `Fork-Feature: fork-docs`.

---

### Task 4: Squash commits keep the PR body; a PR template asks for the slug

**Files:**
- Create: `.github/pull_request_template.md`

**Interfaces:**
- Produces: every squash commit on `main-niu` has the PR body as its message, so a PR's `Fork-Feature:` line becomes the commit's trailer. Task 2's `fold-plan` reads it.

- [ ] **Step 1: Write the template**

`.github/pull_request_template.md`:

```markdown
<!-- What changed and why, in a few sentences. -->

<!-- The feature this folds into at the next sync: one slug from
docs/fork/stack-order.tsv, or a new slug for a new feature. Keep the line
as is; it becomes the squash commit's trailer. -->
Fork-Feature: 
```

- [ ] **Step 2: Human step: switch the squash message to the PR title and body**

Print this and wait for the human to run it:

```
! gh api -X PATCH repos/NorthIsUp/tty7 -f squash_merge_commit_title=PR_TITLE -f squash_merge_commit_message=PR_BODY --jq '{title: .squash_merge_commit_title, message: .squash_merge_commit_message, squash: .allow_squash_merge, merge: .allow_merge_commit, rebase: .allow_rebase_merge}'
```

Expected output: `{"merge":false,"message":"PR_BODY","rebase":false,"squash":true,"title":"PR_TITLE"}`

- [ ] **Step 3: Commit and open the PR**

```bash
git add .github/pull_request_template.md
git commit -m "chore(fork): PR template asks for the Fork-Feature slug" \
  -m "Fork-Feature: fork-infra" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Open the PR with `Fork-Feature: fork-infra` in its body. Once the required checks pass, merge it: `gh pr merge <n> --squash`.

- [ ] **Step 4: Verify the trailer reached the squash commit**

Run: `git fetch -q origin main-niu && git log -1 --format='%(trailers:key=Fork-Feature,valueonly)' origin/main-niu`
Expected: `fork-infra`

---

### Task 5: The fork's CI in its own workflow

**Files:**
- Create: `.github/workflows/niu-ci.yml`
- Modify: `.github/workflows/ci.yml` (restored to upstream's)
- Modify: `.github/workflows/nightly.yml` (restored to upstream's)

**Interfaces:**
- Produces: the required checks `rustfmt`, `build & test (aarch64-apple-darwin)` and `host boundary`, now reported by `niu-ci.yml`.

- [ ] **Step 1: Write the fork workflow from today's fork `ci.yml`**

```bash
git show HEAD:.github/workflows/ci.yml > .github/workflows/niu-ci.yml
sed -i '' '1s/^name: CI$/name: CI (niu)/' .github/workflows/niu-ci.yml
git fetch -q upstream main
git checkout upstream/main -- .github/workflows/ci.yml .github/workflows/nightly.yml
```

Above the `on:` block of `niu-ci.yml`, add this comment:

```yaml
# The fork's CI: upstream's ci.yml, narrowed to macOS and to main-niu. Upstream's
# ci.yml stays byte-identical and disabled here (`gh workflow disable ci.yml`),
# so its edits never conflict with the fork's. Job names match branch
# protection's required checks; keep them.
```

- [ ] **Step 2: Check the workflows and the job names**

Run: `mise x actionlint shellcheck@latest -- actionlint .github/workflows/niu-ci.yml && grep -nE "^    name: (rustfmt|host boundary|build & test)" .github/workflows/niu-ci.yml && git diff --quiet upstream/main -- .github/workflows/ci.yml .github/workflows/nightly.yml && echo IDENTICAL`
Expected: no actionlint output, three name lines (`build & test (${{ matrix.target }})` with the macOS-only matrix), then `IDENTICAL`.

- [ ] **Step 3: Commit and open the PR**

```bash
git add .github/workflows
git commit -m "ci(fork): the fork's CI moves to niu-ci.yml; ci.yml and nightly.yml match upstream" \
  -m "Fork-Feature: fork-infra" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

On the PR, both `CI` and `CI (niu)` run. Confirm `CI (niu)` reports all three required checks green: `gh pr checks <n>`.

- [ ] **Step 4: Human step: disable upstream's ci.yml once the PR has merged**

```
! gh workflow disable ci.yml -R NorthIsUp/tty7 && gh api repos/NorthIsUp/tty7/actions/workflows --jq '.workflows[]|"\(.path) \(.state)"'
```

Expected: `ci.yml disabled_manually`, `niu-ci.yml active`, `nightly.yml disabled_manually`.

---

### Task 6: Fork i18n keys in one block per file

**Files:**
- Modify: `src/ui/i18n/mod.rs`, `src/ui/i18n/en.rs`, `src/ui/i18n/ja.rs`, `src/ui/i18n/zh.rs`

**Interfaces:**
- Consumes: none. `L10nKey` variant names don't change, so no caller changes.

- [ ] **Step 1: Write the failing check**

`mise-tasks/i18n-block-check`:

```bash
#!/usr/bin/env bash
#MISE description="Each i18n file differs from upstream only by one added block"
set -euo pipefail
git fetch -q upstream main
# The fork's base, not upstream's tip: upstream keys added since the last sync are not the fork's.
base=$(git merge-base HEAD upstream/main)
bad=0
for f in src/ui/i18n/mod.rs src/ui/i18n/en.rs src/ui/i18n/ja.rs src/ui/i18n/zh.rs; do
  hunks=$(git diff -U0 "$base" -- "$f" | grep -c '^@@' || true)
  removed=$(git diff -U0 "$base" -- "$f" | grep -c '^-[^-]' || true)
  if [ "$removed" -ne 0 ] || [ "$hunks" -gt 2 ]; then
    echo "$f: $hunks hunks, $removed upstream lines changed or moved"; bad=1
  fi
done
[ "$bad" -eq 0 ] && echo "i18n-block-check: ok"
exit "$bad"
```

`mod.rs` is allowed two hunks: the key block, plus the `// fork` marker line if the macro needs it separate. Each `translate_*` file has one `match`, so its block is one hunk.

- [ ] **Step 2: Run it and see it fail**

Run: `chmod +x mise-tasks/i18n-block-check && mise run i18n-block-check`
Expected: FAIL. `src/ui/i18n/mod.rs` reports several hunks, and its upstream lines changed or moved.

- [ ] **Step 3: Move the fork keys**

In each file, restore every upstream line to upstream's position and text: `git show upstream/main:<file>` is the reference. Collect the fork's keys (and their arms in each `translate_*` match) into one block at the end of the `l10n_keys!` list and at the end of each match, under a line `// fork: keys the fork adds (docs/fork/features/)`. Keep each fork key's text exactly as it is on `main-niu`.

- [ ] **Step 4: Run the check and the i18n tests**

Run: `mise run i18n-block-check && mise x rust@stable -- cargo test --locked --bin tty7-app i18n`
Expected: `i18n-block-check: ok`, then `test result: ok.`

- [ ] **Step 5: Commit and open the PR**

```bash
git add src/ui/i18n mise-tasks/i18n-block-check
git commit -m "refactor(i18n): the fork's keys sit in one block per file" \
  -m "Fork-Feature: fork-config" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: `bundle-macos.sh` overrides go upstream

**Files:**
- Modify (upstream PR): `.github/scripts/bundle-macos.sh`

**Interfaces:**
- Produces: upstream's script honours `TTY7_APP_NAME`, `TTY7_BUNDLE_ID`, `TTY7_BIN_DIR`, `TTY7_DIST`, `TTY7_LOCAL_BUILD_ID`, `ASC_KEY_P8` / `ASC_KEY_ID` / `ASC_ISSUER_ID`, and signing with a keychain identity when no cert is imported. Every default is today's behaviour.

- [ ] **Step 1: Cut the upstream branch**

```bash
git fetch -q upstream main
git worktree add ../tty7-up-bundle -b up/bundle-macos-overrides upstream/main
cd ../tty7-up-bundle
git show origin/main-niu:.github/scripts/bundle-macos.sh > .github/scripts/bundle-macos.sh
```

- [ ] **Step 2: Strip the fork's wording**

Comments that say "the fork" or point at FORK.md become neutral. For example, `# The fork builds the same bundle under its own name and id (FORK.md):` becomes `# A rebranded build sets its own name and id:`. `src/core/fork_update.rs` in the local-build-id comment becomes `the running app can watch it`. Then confirm the defaults are upstream's:

Run: `bash -n .github/scripts/bundle-macos.sh && grep -nE 'TTY7_APP_NAME:-tty7\}|TTY7_BUNDLE_ID:-com.github.tty7\}|TTY7_BIN_DIR:-target/\$\{TARGET\}/release\}|TTY7_DIST:-dist\}' .github/scripts/bundle-macos.sh | wc -l`
Expected: `4`

- [ ] **Step 3: Commit, push and open the upstream PR**

```bash
git commit -am "build(macos): bundle name, id and paths overridable; notarize with an App Store Connect key" \
  -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git push -u origin up/bundle-macos-overrides
gh pr create -R l0ng-ai/tty7 --base main --head NorthIsUp:up/bundle-macos-overrides \
  --title "build(macos): bundle name, id and paths overridable; notarize with an App Store Connect key"
```

The body says what each variable does, states that unset variables keep today's output, and has a checklist where only verified boxes are ticked. Running the script end to end needs signing secrets, so that box stays unticked with that reason.

- [ ] **Step 4: When it merges**

The next sync turns the fork's copy into an empty diff and drops it. Nothing else to do.

---


### Task 8: Build the stack

**Files:**
- Create: `../stack-map.tsv`, outside the repo (one-time, never committed)

**Interfaces:**
- Consumes: `fold-plan` (Task 1), and `sync-upstream` with `SYNC_FROM`, `SYNC_MAP` and `SYNC_SKIP_PUSH` (Task 2). Also `docs/fork/stack-order.tsv` (Task 2) and the feature pages (Task 3).

- [ ] **Step 1: Start from a fresh `main-niu`**

Tasks 1–6 have merged. Then:

```bash
git fetch -q origin main-niu && git fetch -q upstream main
git worktree add ../tty7-build-stack -b build/stack origin/main-niu
cd ../tty7-build-stack
base=$(git merge-base HEAD upstream/main)
```

- [ ] **Step 2: Split the cross-cutting refactors by file**

Find them: `git log --format='%h %s' "$base..HEAD" | grep -E '\((#43|#49|#50|#65)\)$'`

For each one, list its files (`git show --stat --format= <sha>`) and the slug that owns each file, using the spec's stack table. A file whose hunks belong to two slugs goes whole to the earlier slug in `stack-order.tsv`. If every file has the same slug, leave the commit alone; Step 3 maps it whole. Otherwise split it in place:

```bash
sha=<the refactor's full sha>
plan=$(mktemp)
git log --reverse --format='pick %H' "$base..HEAD" | sed "s/^pick $sha\$/edit $sha/" > "$plan"
git -c sequence.editor="cp $plan" rebase -q -i "$base"
git reset -q HEAD^
git add <files owned by the first slug> && git commit -q -m "refactor(<slug>): split from ${sha:0:8}" -m "Fork-Feature: <slug>"
git add <files owned by the next slug> && git commit -q -m "refactor(<slug>): split from ${sha:0:8}" -m "Fork-Feature: <slug>"
git status --short   # must print nothing before continuing
git rebase --continue
```

Repeat for each refactor that needs it. Every commit after a split gets a new sha, so finish all the splits before Step 3.

Run: `git diff --quiet origin/main-niu HEAD && echo SAME_TREE`
Expected: `SAME_TREE`

- [ ] **Step 3: Map every untagged commit to a slug**

```bash
git log --reverse --format='%h%x09%s%x09%(trailers:key=Fork-Feature,valueonly,separator=%x2C)' "$base..HEAD" \
  | awk -F'\t' '$3 == ""' > /tmp/untagged.tsv
```

Write `../stack-map.tsv`: one `short-sha<TAB>slug` line for each row of `/tmp/untagged.tsv`, using the spec's stack table and each commit's files (`git show --stat --format= <sha>`). The fork commits upstream took on 2026-10-01 (#1059–#1064) map to the feature they belong to; the rebase empties and drops them.

Run: `cut -f1 ../stack-map.tsv | sort | uniq -d; comm -13 <(cut -f1 docs/fork/stack-order.tsv | sort -u) <(cut -f2 ../stack-map.tsv | sort -u); comm -23 <(cut -f1 /tmp/untagged.tsv | sort) <(cut -f1 ../stack-map.tsv | sort)`
Expected: no output. No sha is mapped twice, every slug is in the order file, and every untagged commit is mapped.

- [ ] **Step 4: Fold, rebase and test, without pushing**

Run: `SYNC_FROM=build/stack SYNC_MAP=$PWD/../stack-map.tsv SYNC_SKIP_PUSH=1 mise run sync-upstream`
Expected: it ends `ready: git push --force-with-lease=main-niu:<sha> origin HEAD:main-niu`. Note the `<sha>`: Step 5 uses it as the lease. The fold's tree check passed, so the regrouping changed nothing. `mise run test` passes, apart from the environment failures recorded in `docs/fork/TODO.md`.

Run: `git log --reverse --format=%s upstream/main..HEAD`
Expected: only `fork(<slug>): …` lines, at most one per slug, in `stack-order.tsv` order.

Run: `git log --format=%s upstream/main..HEAD | grep -cv '^fork('`
Expected: `0`

- [ ] **Step 5: Human step: back up and publish**

Print and wait:

```
! git push origin origin/main-niu:refs/heads/backup/main-niu-pre-stack && git push --force-with-lease=main-niu:<lease sha from Step 4> origin HEAD:main-niu
```

If the lease fails because a PR merged meanwhile, rerun Step 4 without `SYNC_SKIP_PUSH`, keeping the same `SYNC_FROM` and `SYNC_MAP`. The sync refolds the new commit and retries the push itself.

- [ ] **Step 6: Verify on GitHub, then clean up**

Run: `gh run list -R NorthIsUp/tty7 --branch main-niu --workflow niu-ci.yml -L 1 --json conclusion,headSha --jq '.[0]'`
Expected: once the run finishes, `"conclusion":"success"` for the pushed sha.

Run: `git fetch -q origin && git log --format=%s upstream/main..origin/main-niu | grep -cv '^fork('`
Expected: `0`

Then `rm ../stack-map.tsv` and `git worktree remove ../tty7-build-stack`.
