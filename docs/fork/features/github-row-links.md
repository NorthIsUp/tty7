# the GitHub panel's Open in Graphite tile

`Fork-Feature: github-row-links`. The commit that carries this feature is `fork(github-row-links): …` on `main-niu`.

Upstream took the row links and pull request stacks (l0ng-ai/tty7#1116). What stays here is the Open in Graphite tile a hovered pull request row shows beside Open on GitHub, always on; upstream review of it as an opt-in setting is l0ng-ai/tty7#1177. The Session tab and its mentions-first order feed the same stack drawing as the lists.

## Fork-owned files

| file | what it holds |
|---|---|
| `assets/icons/graphite.svg` | Graphite's mark, from its web app's favicon |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/panel_github.rs` | `hovered_links` takes the slug and adds the Graphite tile; `graphite_url`, `link_tile` | the tile |
| `src/ui/github_session.rs` | `github_session_tab_body` draws through `github_stack_rows`; test `Item`s gain `pull` | the Session list stacks too |
| `src/ui/github/merge.rs` | reads `pull.refs.base_ref` | `PullInfo`'s branches moved into `refs` |
| `src/ui/assets.rs` | `icons/graphite.svg` | the tile's icon |
| `src/ui/i18n/{mod,en,ja,zh,ru}.rs` | `GitHubOpenInGraphite` | the tile's tooltip |
| `docs/window/side-panel.mdx` | GitHub bullets | the tile |
