# a config.json from before ForkConfig loads and saves every fork key

`Fork-Feature: fork-config-compat`. The commit that carries this feature is `fork(fork-config-compat): …` on `main-niu`.

Tests only, at the tail because they name every feature's settings: a file written before `ForkConfig` existed loads every fork key and saves it back unchanged, the retired keys (`github_panel_session_filter`, `github_panel_default_list`, `continue_stagger_ms`) still load, and a bad value for one fork key falls back without failing the file.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/fork_config.rs` | `tests`: `OLD_FILE`, `an_old_file_loads_and_saves_the_same_top_level_keys`, `retired_keys_still_load`, `a_bad_fork_value_falls_back_without_failing_the_file` | the cross-feature config contract |
