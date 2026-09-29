# T3 — Pane nice (F3)

Read `docs/fork/spec.md` §F3 and `docs/fork/master-plan.md` first.

- New `crates/tty7-core/src/daemon/nice.rs` (declared in the daemon module list):
  - `pub(crate) fn clamp(n: i32) -> Option<i32>`: `None` for 0, else `n.clamp(0, 19)`,
    and `None` if the clamped value is 0;
  - `#[cfg(unix)] pub(crate) fn apply(pid: u32)` reads `Config::load().nice`,
    and on `Some(n)` calls `libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, n)`,
    logging a warning on failure. On `#[cfg(not(unix))]` it is a no-op.
    Check that `libc` is already a dependency of tty7-core, and do not add a
    crate.
- Hook: in `crates/tty7-core/src/daemon/pane.rs` `DaemonPane::spawn`, right
  after `let shell_pid = child.process_id();` (~:1616), add
  `if let Some(pid) = shell_pid { super::nice::apply(pid); }` (adapt the path).
  That is the only upstream edit, and it goes in `FORK.md`.
- Tests: the `clamp` table (0, 5, -3, 40), plus a unix test that spawns
  `sleep 5` via std `Command`, applies with n=7 through an internal
  `apply_n(pid, n)`, and reads back `libc::getpriority` == 7.
- Docs: one line in `docs/reference/configuration.mdx` already exists
  (the `nice` row). Add nothing else.
