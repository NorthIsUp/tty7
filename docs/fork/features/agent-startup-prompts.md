# answers an agent's startup prompts and offers to restart a draining Codex server

`Fork-Feature: agent-startup-prompts`. The commit that carries this feature is `fork(agent-startup-prompts): …` on `main-niu`.

## Startup prompts

An agent that starts in a pane this view spawned is watched for `STARTUP_WINDOW` (10s), past its exit: the first of its `CLIAgent::startup_prompts` whose `screen` regex matches the pane's visible text is answered, once, and the watch ends. Each answer logs `agent startup prompt answered: <agent> <name>`, never the screen. An `Answer::Keys` is typed while the agent still runs; an `Answer::Offer` raises a notice whose button is the only thing that acts.

- `resume-cwd`: codex 0.166's directory picker on `codex resume` from another directory, answered with Esc ("esc use session"), since `1` only moves its cursor.
- `app-server-draining`: every `codex resume` dies with `Server is draining … serverShuttingDown` while Codex's shared app-server drains. One notice per incident, however many panes hit it, offers Restart Codex server: `codex app-server daemon restart`, then each of those panes' sessions resumed. Restarting cuts off running Codex sessions, so it never runs unclicked. Local panes only; offers in the 30s after a restart are dropped, since its own resumes can still show the old error.

| file | function / site | why |
|---|---|---|
| `crates/tty7-core/src/core/cli_agent.rs` | `CLIAgent::startup_prompts`, `StartupPrompt`, `Answer`, `Offer`, `StartupWatch` (+ tests) | the per-agent table and its window |
| `src/terminal/view.rs` | `TerminalView::startup_watch`, armed in `poll_agent_detection` beside `AgentDetected`; `answer_startup_prompt`, `StartupOffer` (+ test) | the 300ms foreground poll reads the screen and writes the keys or emits the offer |
| `src/ui/app.rs` | the pane subscriptions | `StartupOffer` → `agent_resume::offer` |
| `src/ui/agent_resume.rs` | `offer`, `CodexDrain`, `restart` | the one notice per incident, the restart and the resumes |
| `src/ui/first_prompt.rs` | `on_this_machine` made `pub(crate)` | an offer is for this machine's Codex only |
| `src/ui/i18n/{mod,en,zh,ja,ru}.rs` | after `AppPaneNoCodingAgent` | `AppCodexServerDraining`, `AppCodexServerRestart`, `AppCodexServerRestartFailed` |
