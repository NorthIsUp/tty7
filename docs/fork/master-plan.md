# Master plan: clawmux parity (F1–F4)

Spec: `docs/fork/spec.md`. Integration branch: `feat/new-tab-page`.

## Global Constraints (copied into every task; current text wins)

- Fork logic goes in fork-owned files. An upstream file gets only a short hook
  call, ideally at most 10 changed lines, and every hook is added to `FORK.md`.
  Never reformat or move upstream code.
- Build and test only through mise: `mise run build`, `mise run test -p <crate>`,
  and `mise run test --bin tty7-app -- -- <filter>` (mise eats a bare `--`).
  `cargo fmt` before committing.
- Comments state only the non-obvious why. Match tty7's doc-comment voice. No
  inline imports.
- Every new user-facing string is an `L10nKey` with en, zh and ja arms; the
  i18n coverage tests must pass.
- Every new keymap action follows the `ContinueAllAgents` pattern:
  `core/actions.rs`, `keymap.rs` (the three sites), `search/command.rs` (the
  four sites), `app.rs` (dispatch plus `on_action`), and i18n.
- These tests fail on untouched upstream too; ignore them and do not "fix"
  them: `terminal::history::*` (they read the real HOME), `ui::scm::detail::*`
  (they need `GIT_CONFIG_GLOBAL=/dev/null`), `daemon::spawn::tests::reap_guard_*`,
  and `cli_e2e exec_returns_*`. Run the suites with `GIT_CONFIG_GLOBAL=/dev/null`.
- Never `--no-verify`. Never commit anything under `target/`.

## Shared Contracts (landed in 76892ab6, read-only for tasks)

```rust
// crates/tty7-core/src/core/fork_config.rs — ForkConfig, flattened into Config
// (read as `cfg.fork.new_tab_page`)
pub new_tab_page: bool,                            // default true
pub dir_roots: Vec<String>,                        // default ["~/src","~/code","~/projects"]
pub dir_frecency: HashMap<String, ProfileUsage>,   // key: absolute path
pub group_colors: HashMap<String, String>,         // group name -> "#rrggbb"
pub nice: i32,                                     // default 5; 0 = off
```

Existing APIs to reuse, never reimplement:

```rust
ui::search::score::fuzzy_score(query: &str, text: &str) -> Option<i32>
ui::home::display_path(path, home) // "~/src/x"
Tty7App::offered_agents(&self, cx: &App) -> Vec<CLIAgent>          // ui/agent_launch.rs
agent_launch::most_recent(offered: &[CLIAgent], usage) -> Option<CLIAgent>
Tty7App::new_tab_slot(cwd, shell, window, cx) -> Option<PaneSlot>  // ui/app.rs
ProfileUsage { count, last_used }.score(now)                       // core/config.rs
Tty7App::update_config(cx, |cfg| ..)                               // persists config.json
```

## Pinned decisions

| id | decision |
|---|---|
| C1 | The new tab page is a modal picker owned by `Tty7App` (like `switcher`), not a new `PaneSlot` variant. |
| C2 | Group colour = override, else FNV-1a(name) into the theme's `ansi16` [1–6, 9–14]. |
| C3 | nice = `setpriority` on the shell pid after spawn, in the daemon. |
| C4 | Fork upkeep = `FORK.md` + `mise run sync-upstream`, no scheduled CI (fork Actions are off). |

## Review Focus (each pinned by a test in its owning task)

1. The picker's candidate list: dedupe by path, `~` expansion, missing roots
   and non-directories skipped, and an exact typed path offered first (T1).
2. `new_tab_page: false` gives upstream New Tab exactly (T1).
3. Esc opens nothing and bumps no frecency (T1).
4. The same group name gets the same colour; a bad hex override falls back (T2).
5. nice is clamped and never fails a spawn (T3).

## Sections / tasks

The per-task plans (`docs/fork/plan/`) and the run log were one-shot and
are deleted; git history has them.

| id | scope | touches |
|---|---|---|
| T1 | F1 | new `src/ui/new_tab_page.rs`; hooks in `app.rs` (new_tab, render, field), `agent_launch.rs` (launch with explicit cwd), `ui/mod.rs`; i18n en/zh/ja/mod; `docs/window/` page |
| T2 | F2 | new `src/ui/group_color.rs`; hook in `tab_sidebar.rs` header; `ui/mod.rs` |
| T3 | F3 | `crates/tty7-core/src/daemon/pane.rs` (a call after spawn) + a small fn in a new `daemon/nice.rs` |
| T4 | F4 | `FORK.md`, `mise.toml`; runs last, so it lists T1–T3's hooks |

Hot files: `src/ui/mod.rs` (T1 and T2 each add one `pub mod` line; a trivial
merge). Waves: [T1, T2, T3], then [T4].
