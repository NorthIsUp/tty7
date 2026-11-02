# the GitHub panel's pull request merge buttons and auto-merge

`Fork-Feature: github-merge`. The commit that carries this feature is `fork(github-merge): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/core/github/merge.rs` | `MergeMethod`, `MergeRules` and `merge_rules` (`GET /repos/{owner}/{repo}`'s `allow_*`; unsaid means every method, no auto-merge), `merge` (`PUT /pulls/{n}/merge`), `set_auto_merge` (GraphQL `enable`/`disablePullRequestAutoMerge`, whose `errors` become `ApiError::Rejected`) |
| `src/ui/github/merge.rs` | the buttons under an open, non-draft pull request's head: one per allowed method, then auto-merge on (first allowed method) or off. Rules read once per repository. Merging and enabling ask through `window.prompt`; the call runs on `off_ui`, success marks the detail due, failure shows `describe_error` |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/github/api.rs` | `Transport::put` (default refuses with 405), `ApiError::Rejected`, `classify` (a 4xx with a `message` is `Rejected`), `decode` made `pub(super)` | the merge is a PUT; GitHub's reason for refusing one is worth showing |
| `crates/tty7-core/src/core/github/http.rs` | `HttpTransport::put`, `request` takes the write's method | |
| `crates/tty7-core/src/core/github/accounts.rs` | `FallbackTransport::put` | |
| `crates/tty7-core/src/core/github/model.rs` | `PullInfo::{node_id, auto_merge}`, `RawPull`'s | GraphQL keys auto-merge by node id; whether it is on picks the button |
| `crates/tty7-core/src/core/github/mod.rs` | `pub mod merge` | |
| `src/ui/github/mod.rs` | `mod merge`, `GitHubPanelState::merges` | |
| `src/ui/github/detail.rs` | `render_github_detail` (`github_merge_box` after the head) | next to the merge state it acts on |
| `src/ui/panel_github.rs` | `describe_error`'s `Rejected` arm | GitHub's own words |
| `src/ui/i18n/{mod,en,ja,zh,ru}.rs` | `GitHubMergeCommit`, `GitHubMergeSquash`, `GitHubMergeRebase`, `GitHubAutoMergeEnable`, `GitHubAutoMergeDisable`, `GitHubMergeConfirm`, `GitHubAutoMergeConfirm` | the feature's words |
| `docs/window/side-panel.mdx` | GitHub intro, detail bullets | the merge buttons |
