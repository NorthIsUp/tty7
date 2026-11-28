# a Claude subagent's cwd stays off the pane

`Fork-Feature: claude-subagent-cwd`. The commit that carries this feature is `fork(claude-subagent-cwd): …` on `main-niu`.

Carries upstream [#1265](https://github.com/l0ng-ai/tty7/pull/1265) until it merges; sync drops this commit then.

## Hooks in upstream files

| file | why |
|---|---|
| `crates/tty7-core/src/core/agent_hooks.rs` | upstream #1265 |
