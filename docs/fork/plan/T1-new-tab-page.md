# T1 — New tab page (F1)

Read `docs/fork/spec.md` §F1 and `docs/fork/master-plan.md` first (Global
Constraints win).

## Shape

- New file `src/ui/new_tab_page.rs`, declared as `pub mod new_tab_page;` in
  `src/ui/mod.rs`. It holds:
  - `pub(crate) struct NewTabPage { query: Entity<InputState>, kinds: Vec<Kind>, kind: usize, rows: Vec<DirRow>, selected: usize, focus: FocusHandle, .. }`,
    with `enum Kind { Terminal, Agent(CLIAgent) }`.
  - `pub(crate) fn candidates(active: Option<&Path>, tab_cwds: &[PathBuf], frecency: &HashMap<String, ProfileUsage>, roots: &[String], home: Option<&Path>, now: u64) -> Vec<PathBuf>`.
    This is pure (filesystem reads only) and unit-tested with a `tempfile::TempDir`
    tree. It dedupes by canonical path, skips missing entries, non-directories
    and dot-directories, expands `~`, and keeps the source order from the spec.
  - `pub(crate) fn filter(query: &str, dirs: &[PathBuf], home: Option<&Path>) -> Vec<PathBuf>`
    (pure, tested). It scores with `fuzzy_score` against the `~`-form display
    path. A query that names an existing directory goes first.
  - `impl Tty7App { pub(crate) fn open_new_tab_page(..) -> bool; fn close_new_tab_page(..); fn commit_new_tab_page(..); pub(crate) fn render_new_tab_page(..) -> Option<AnyElement> }`.
    `open_new_tab_page` returns false (so the caller falls through to upstream)
    when `!cfg.new_tab_page` or the workspace is remote.
- Hooks in upstream files (keep each small; list every one in your result for `FORK.md`):
  - `app.rs`:
    - a field `pub(crate) new_tab_page: Option<crate::ui::new_tab_page::NewTabPage>`, initialised `None`;
    - in `new_tab()`, first line: `if self.open_new_tab_page(window, cx) { return; }`;
    - render the overlay where the switcher's overlay is rendered (find `render_switcher` in `render`).
  - `agent_launch.rs`: split `launch_agent` so the NewTab arm's cwd comes from
    a caller-supplied `Option<PathBuf>`. Add `launch_agent_in(agent, cwd, window, cx)`,
    which `launch_agent(.., SpawnWhere::NewTab, ..)` calls with the active
    tab's cwd. The behaviour is identical for existing callers.
- UI: reuse the switcher's card metrics and colours (`switcher::CARD_TOP`,
  `Surfaces`, `cx.theme()`) and home.rs's row style (28px rows, `ROW_RADIUS`,
  hover). Kind chips form one row of small buttons, and the selected one is
  filled with `theme.primary`. Keys: ↑/↓, Tab/⇧Tab, ⌃1..⌃9, Enter, Esc; the
  query input keeps focus.
- Enter:
  - Terminal → `new_tab_with_cwd(Some(dir), None, ..)` (make it `pub(crate)` if needed);
  - Agent → `launch_agent_in(agent, Some(dir), ..)`;
  - then `update_config(|c| { let u = c.dir_frecency.entry(path).or_default(); u.count += 1; u.last_used = unix_now(); })`.
- i18n keys: `NewTabPageTitle` ("New tab"), `NewTabPageTerminal`
  ("Terminal"), `NewTabPagePlaceholder` ("Directory…"), `NewTabPageHint`
  ("Enter opens · Tab switches kind · Esc cancels"), each with zh and ja arms.
- Docs: `docs/window/new-tab-page.mdx` (short), linked from `docs/docs.json` if the nav lists pages.

## Tests (TDD; write them first)

- `candidates`:
  - order: active first, then the other tabs, frecency, root children;
  - duplicates collapse;
  - a root missing, a file under a root and `.hidden` are all skipped;
  - `~` expands.
- `filter`: an empty query keeps the order, a fuzzy match ranks, and an exact
  existing dir typed as `~/x` comes first.
- `new_tab_page: false` → `open_new_tab_page` returns false. A gpui test is
  good if cheap (see `dispatching_new_window_opens_a_second_window_beside_the_first`
  in app.rs); otherwise test the gate as a pure fn.
- Esc path: no frecency change. Put the bump behind a pure helper and assert
  it is only called from commit.

## Done when

`mise run build` is clean. The new tests pass, along with the i18n and keymap
suites in `mise run test --bin tty7-app` (known env failures excepted). The
result lists the hook points.
