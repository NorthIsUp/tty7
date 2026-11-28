# OSC 52 clipboard writes reach the clipboard

`Fork-Feature: osc52-clipboard`. The commit that carries this feature is `fork(osc52-clipboard): …` on `main-niu`.

Carries upstream [#1337](https://github.com/l0ng-ai/tty7/pull/1337) until it merges; sync drops this commit then.

## Hooks in upstream files

| file | why |
|---|---|
| `crates/tty7-core/src/core/clipboard.rs` | upstream #1337 |
| `src/terminal/remote.rs` | upstream #1337 |
| `src/terminal/view.rs` | upstream #1337 |
