# the fork's settings section in config.json

`Fork-Feature: fork-config`. The commit that carries this feature is `fork(fork-config): …` on `main-niu`.

The one fork-only commit a feature may build on: an empty `ForkConfig`, flattened into upstream's `Config`, so a fork setting sits at the top level of `config.json` while upstream's `Config` carries one fork line. Each feature adds its own fields, defaults and enums to it. Upstreaming a feature moves its fields into `Config` and turns `cfg.fork.x` into `cfg.x`.

## Fork-owned files

| file | what it holds |
|---|---|
| `crates/tty7-core/src/core/fork_config.rs` | `ForkConfig` and its `Default`; the test that no fork key is also an upstream key |

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/config.rs` | `Config::fork` (`#[serde(flatten)]`), `Default` | every fork setting lives in `fork_config::ForkConfig`, at the top level of `config.json` |
| `crates/tty7-core/src/core/mod.rs` | module list | `pub mod fork_config` |
