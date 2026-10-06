# the GitHub panel's detail fetches in parallel

`Fork-Feature: github-parallel`. The commit that carries this feature is `fork(github-parallel): …` on `main-niu`.

A pull request's detail sends its seven requests as three round trips instead of seven: issue ‖ comments, then `/pulls/{n}` → (check-runs ‖ status) alongside reviews ‖ files. `checks` sends its two requests together, which also halves the checks poll's latency. Scoped threads; the request count is unchanged.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/github/api.rs` | `detail`, `checks`, `join` | the parallel fetch |
| `crates/tty7-core/src/core/github/model.rs` | `RawPull::head_sha` | checks start before the pull request is turned into an item |
