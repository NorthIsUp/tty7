# the GitHub panel's Session tab and This session filter

`Fork-Feature: github-session`. The commit that carries this feature is `fork(github-session): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/github_session.rs` | the GitHub tab's Session list, its default: every issue and pull request the focused pane's agent session mentions, latest first, rows from the lists, details and a capped one-at-a-time lookup; a mention shows only once a real item answers it (nothing above the highest number held is looked up, a 404 drops it); `remote_pick`, the fork before upstream; `ListSort`, the Issues and Pull Requests lists' Recent \| Number order |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/github/mod.rs` | `GitHubPanelState::session`, `github_refresh`, `off_ui` made `pub(crate)` | the Session tab's state (on by default) and mentions cache; refresh marks it due; the lookup runs on `off_ui` |
| `crates/tty7-core/src/core/github/model.rs` | `StateFilter::All`, `StateFilter::admits` | the Open \| Closed \| All switch; which rows the Session list keeps |
| `crates/tty7-core/src/core/github/api.rs` | `ListQuery::by_number`, `list_request`'s `sort` | Number: GitHub sorts by created, so by number, across pages |
| `src/ui/github/mod.rs` | `github_query` | `by_number` from the Recent \| Number switch |
| `crates/tty7-core/src/core/github/api.rs` | `item` | one issue or pull request as a list row (`/issues/N`), for a mention no list page has |
| `src/ui/panel_github.rs` | test `the_tab_lists_and_opens_an_issue_without_touching_the_network`, test imports | `github_panel_prefer_origin = false`: it checks upstream's own remote order; `session.tab = false`: it checks the Issues list |
| `src/ui/github/mod.rs` | `github_target_for` | `github_session::remote_pick`: `origin` before `upstream` unless the user picked one |
| `src/ui/panel_github.rs` | `render_panel_github` (list ensure, body), `github_switch_row` (kind cells, sort cells, states), `switch_cell` / `github_item_row` made `pub(crate)`, `host.clone()` into `github_branch_pull`, `github_list_body` (takes `host`, `window`; `mentions_first`) | no kind list fetched on the Session tab; `github_session_tab_body` before the plain list; the Session cell before Issues \| Pull Requests, which light only off it; All after Open \| Closed; the Recent \| Number switch, Recent lifting the session's mentions |
| `src/ui/i18n/{mod,en,ja,zh}.rs` | `GitHubSession`, `GitHubAll`, `GitHubNoSessionMentions`, `GitHubNoSessionMatches`, `GitHubSortRecent`, `GitHubSortNumber` | the feature's words |
| `docs/window/side-panel.mdx` | GitHub list bullets | the Session tab, All, Recent \| Number |
