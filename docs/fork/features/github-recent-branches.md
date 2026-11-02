# the GitHub panel keeps the last five branches

`Fork-Feature: github-recent-branches`. The commit that carries this feature is `fork(github-recent-branches): …` on `main-niu`.

Under the repository row the panel shows the branch the pane is on, and below it
the four it was on before, newest first, read from HEAD's reflog. Each carries its
pull request when it has one, one click from its detail; a branch without one is
its name alone. The list is re-read with the branch, and at once after a switch.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/github/mod.rs` | module list; `GitHubState::recent`; `BRANCH_TTL` `pub(crate)`; `github_branch_pull` split, its fetch now `github_pull_for(key)` | the cache, and a pull request lookup for any branch |
| `src/ui/panel_github.rs` | `render_github`'s pinned block; `github_branch_pull_row` takes `Option<&Item>` and `this_branch`, its id carries the number | the current branch always, then the recent ones, each a caption with an optional row |
