# T4 — Fork upkeep (F4)

Runs after T1–T3 merged. Read `docs/fork/spec.md` §F4.

- `FORK.md` at the repo root:
  - one paragraph on what the fork is and the rebase rule (fork logic in
    fork-owned files, upstream files get hook calls only);
  - a table of fork-owned files;
  - a table of every hook in an upstream file (file · function/site · why).
  Build it from `git diff upstream/main --stat` and by reading each upstream
  file's hunks (`git diff upstream/main -- <file>`). Cover PR #1's hooks too:
  app.rs restore/wake/continue, agent_launch minting, daemon argv adoption,
  config fields, keymap/command/i18n for ContinueAllAgents, main.rs
  `--continue`, and pending_pane `agent_prompt`.
- `mise.toml`: a task `sync-upstream`:
  `git remote get-url upstream >/dev/null 2>&1 || git remote add upstream https://github.com/l0ng-ai/tty7.git; git fetch upstream main && git rebase upstream/main || { echo "conflict: resolve using FORK.md's hook table, then git rebase --continue"; exit 1; }`.
  Use the usage-spec style if args are ever needed; this one takes none.
- Verify: `mise tasks` lists it; run `git fetch upstream main` only (no rebase)
  to prove the remote works; and check `FORK.md`'s table against
  `git diff upstream/main --name-only`: every changed upstream file appears.
