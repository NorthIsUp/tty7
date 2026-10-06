# the GitHub panel's row links and pull request stacks

`Fork-Feature: github-row-links`. The commit that carries this feature is `fork(github-row-links): …` on `main-niu`.

A hovered row in the Session, Issues and Pull Requests lists shows an Open on GitHub tile and, for a pull request, Open in Graphite; both stop the click so the row does not also open its detail. Pull requests whose base branch is another listed pull request's head (in the same repository, not a fork) are gathered into one run, each above its base, with a connector through their glyphs that ends under the bottom one in a dot and the stack's base branch, so the run reads bottom-up. The branches come from `/pulls`, which the Pull Requests tab and the Session tab's `state=all` list already read; a row from `/issues` (a label-filtered list, a one-off Session lookup) has none and is never stacked.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/core/github/stack.rs` | `PullRefs` (head ref, head owner from `head.label`, base ref), `stack_order` (cycle-safe grouping and order, with each row's `StackLink`), `graphite_url` |
| `assets/icons/graphite.svg` | Graphite's mark, from its web app's favicon, for the Open in Graphite tile |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/github/mod.rs` | `pub mod stack` | |
| `crates/tty7-core/src/core/github/model.rs` | `Item::pull`, `RawBranchRef::label` (+ `Default`), `RawPull::into_item` fills `pull`, `RawIssue::into_item` leaves it `None` | the branches ride on the row |
| `crates/tty7-core/src/core/github/api.rs` | `detail` copies `pull` from `/pulls/{n}` | a detail's row stacks like a list's |
| `src/ui/panel_github.rs` | `github_list_body` and the Session tab draw through `github_stack_rows` (`stack_order` plus `stack_trunk` under each stack's bottom row), `github_item_row` takes a `StackLink`, `stack_glyph`, `hovered_links`, `github_tile` takes any `ElementId` | the drawing and the tiles |
| `src/ui/github_session.rs` | `github_session_tab_body` runs `stack_order`; test `Item`s gain `pull` | the Session list stacks too |
| `src/ui/assets.rs` | `icons/graphite.svg` | the tile's icon |
| `src/ui/i18n/{mod,en,ja,zh,ru}.rs` | `GitHubOpenInGraphite` | the Graphite tile's tooltip |
| `docs/window/side-panel.mdx` | GitHub bullets | the tiles and stacks |
