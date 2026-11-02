# Search Everywhere's Text tab finds text in files

`Fork-Feature: search-text`. The commit that carries this feature is `fork(search-text): …` on `main-niu`.

## Fork-owned files

| file | what it holds |
|---|---|
| `src/ui/search/text.rs` | Search Everywhere's Text tab: find in files over `Host::search_content`, debounced, never on All; `LiveTab`, the per-palette ask, debounce and latest-answer check History shares |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/search/mod.rs` | `SearchTab`, `ORDER`, `title`, `placeholder`, module list, `CARD_MAX_W` re-export (+ test) | the `Text` tab, on the row between Hosts and Actions; the `History` tab, after Sessions; the `NewTab` tab, off the row |
| `src/ui/search/command.rs` | `CommandKind`, `id`, `key_spec`, `actions` | `SearchText` (Search Text in Files…) |
| `src/ui/search/sources.rs` | `Catalog` fields, `new`, `source`, `all` | `live` (`text::LiveTab`s); `all` leaves Text and History out; New Tab has no rows of its own; `Sessions::highlights` leaves open sessions to the Terminals rows |
| `src/ui/search/view.rs` | `perform_search`, `set_tab`, `render_empty`, `update_catalog` visibility | `Catalog::ask_live` for the tab showing; the too-short and remote hints |
| `src/ui/panel_search.rs` | module list | `pub(crate) mod model` for `split_relative` |
| `src/ui/app.rs` | `open_search` | `catalog.live = palette_live_tabs(..)` |
