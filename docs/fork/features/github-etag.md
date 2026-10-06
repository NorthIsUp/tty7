# the GitHub panel's reads revalidate by ETag

`Fork-Feature: github-etag`. The commit that carries this feature is `fork(github-etag): …` on `main-niu`.

`HttpTransport` remembers the last answer and ETag of each plain GET and asks again with `If-None-Match`. A 304 replays the remembered answer, and a signed-in 304 does not count against the rate limit, so the 120s revalidations and the 20s checks poll stop spending the hourly allowance the panel shares with `gh`. `get_full` is never cached: its signed attachment URLs expire.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/github/http.rs` | `Etags`, `HttpTransport::etags`, `request` sends `If-None-Match` and replays on 304 | the whole feature; callers are untouched |
