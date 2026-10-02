# The side panel's current tab is lit and stays put

`Fork-Feature: panel-tabs`. The commit that carries this feature is `fork(panel-tabs): …` on `main-niu`.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/tab_strip.rs` | `right_panel_tabs` | the current tab gets a pill; clicking it does nothing, so only the close button hides the panel |
| `docs/window/side-panel.mdx` | intro | says so |
