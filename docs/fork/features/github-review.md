# the GitHub panel's pull request review box

`Fork-Feature: github-review`. The commit that carries this feature is `fork(github-review): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/core/github/review.rs` | `ReviewEvent` (Comment \| Approve) and `submit_review`: `POST /repos/{owner}/{repo}/pulls/{n}/reviews` with `{body, event}` |
| `src/ui/github/review.rs` | the review box under a pull request's conversation: a draft per pull request, Comment (needs text) and Approve, the submit on `off_ui`; success drops the draft and marks the detail due, failure shows `describe_error` |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/github/api.rs` | `Transport::post` (default refuses with 405), `repo_path` made `pub(super)` | the one write; the test fakes stay read-only untouched |
| `crates/tty7-core/src/core/github/http.rs` | `HttpTransport::post`, `request` takes an optional body, `headers` | a POST with the same headers, token and error mapping as a GET |
| `crates/tty7-core/src/core/github/accounts.rs` | `FallbackTransport::post` | a review goes through the account that can see the repository |
| `crates/tty7-core/src/core/github/mod.rs` | `pub mod review` | |
| `src/ui/github/mod.rs` | `mod review`, `GitHubPanelState::reviews` | the drafts |
| `src/ui/github/detail.rs` | `render_github_detail` (`window` used, `github_review_box` after the comments) | the box sits where GitHub's does |
| `src/ui/i18n/{mod,en,ja,zh,ru}.rs` | `GitHubReviewPlaceholder`, `GitHubReviewComment`, `GitHubReviewApprove` | the feature's words |
| `docs/window/side-panel.mdx` | GitHub intro, detail bullets | the review box |
