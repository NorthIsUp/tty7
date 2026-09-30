//! Quick launch for the coding agents this machine can run (#955).
//!
//! Launching an agent never types into a pane that already exists. It opens a
//! new shell — a tab in the active tab's directory, or a split when the entry
//! point was asked for one — and types the agent's command line into *that*
//! shell once it is there. The agent then runs the way a hand-typed one does:
//! the daemon detects it from the pane's foreground, status and resume work as
//! usual, and quitting it leaves the shell behind.
//!
//! Which agents are offered: on this computer, those whose launch program is
//! on `PATH`. A remote workspace's `PATH` is not something the `Host` trait can
//! answer — it has no way to run a command or read the far shell's
//! environment — so there the list is the agents seen running in that
//! workspace before (`WindowView::seen_agents`). Either way the order is
//! frecency, recorded the way saved SSH hosts' is.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, Axis, Context, Entity, Window};
use gpui_component::WindowExt as _;

use crate::core::cli_agent::{CLIAgent, launch_program, program_on_path};
use crate::core::config::{Config, ProfileUsage, unix_now};
use crate::core::session::WorkspaceStore;
use crate::terminal::view::TerminalView;
use crate::ui::app::{SpawnAs, SpawnWhere, Tty7App, join_shell_args};
use crate::ui::i18n::{L10nKey, t_fmt};
use crate::ui::pane::PaneSlot;

/// How the keymap spells "launch this agent": `LaunchAgent:claude`.
const LAUNCH_ACTION_PREFIX: &str = "LaunchAgent:";

/// One keymap action name per agent, in [`CLIAgent::ALL`] order.
pub(crate) fn launch_action_names() -> &'static [(CLIAgent, String)] {
    static NAMES: LazyLock<Vec<(CLIAgent, String)>> = LazyLock::new(|| {
        CLIAgent::ALL
            .into_iter()
            .map(|agent| (agent, format!("{LAUNCH_ACTION_PREFIX}{}", agent.slug())))
            .collect()
    });
    &NAMES
}

pub(crate) fn launch_action_name(agent: CLIAgent) -> &'static str {
    launch_action_names()
        .iter()
        .find(|(a, _)| *a == agent)
        .map(|(_, name)| name.as_str())
        .expect("every agent has a launch action")
}

pub(crate) fn agent_for_launch_action(action: &str) -> Option<CLIAgent> {
    action
        .strip_prefix(LAUNCH_ACTION_PREFIX)
        .and_then(CLIAgent::from_slug)
}

/// The agents whose launch program is on `path`, in [`CLIAgent::ALL`] order.
/// An agent with an `agent_launch` override is looked up by the override's
/// program, since that is what the launch will run.
pub(crate) fn installed_on(
    path: &std::ffi::OsStr,
    overrides: &HashMap<String, String>,
) -> Vec<CLIAgent> {
    CLIAgent::ALL
        .into_iter()
        .filter(|agent| {
            launch_program(&agent.launch_command(overrides))
                .is_some_and(|program| program_on_path(&program, path))
        })
        .collect()
}

/// `agents` most-used-first by frecency score; agents with the same score
/// keep the order they came in.
pub(crate) fn by_frecency(
    agents: impl IntoIterator<Item = CLIAgent>,
    usage: &HashMap<String, ProfileUsage>,
    now: u64,
) -> Vec<CLIAgent> {
    let score = |agent: &CLIAgent| usage.get(agent.slug()).map(|u| u.score(now)).unwrap_or(0.0);
    let mut agents: Vec<CLIAgent> = agents.into_iter().collect();
    agents.sort_by(|a, b| {
        score(b)
            .partial_cmp(&score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    agents
}

/// The agent "New Agent Tab" opens: of `offered`, the one launched or seen
/// most recently; the first offered when none has been used yet.
pub(crate) fn most_recent(
    offered: &[CLIAgent],
    usage: &HashMap<String, ProfileUsage>,
) -> Option<CLIAgent> {
    let last_used = |agent: &CLIAgent| usage.get(agent.slug()).map_or(0, |u| u.last_used);
    offered
        .iter()
        .copied()
        .filter(|agent| last_used(agent) > 0)
        // `max_by_key` keeps the last of equals; reversed, that is the first.
        .rev()
        .max_by_key(last_used)
        .or_else(|| offered.first().copied())
}

/// The `agent_launch` line "Set Current Launch Args as Default" writes for a
/// pane that `agent` is running in with `argv`.
pub(crate) fn launch_line_to_save(agent: CLIAgent, argv: &[String]) -> Option<String> {
    agent
        .launch_argv_for_default(argv)
        .map(|argv| join_shell_args(&argv))
}

/// Make `line` the command `agent` is launched with from now on.
pub(crate) fn remember_launch_line(cfg: &mut Config, agent: CLIAgent, line: String) {
    // One entry per agent, whatever case an existing key was written in.
    cfg.agent_launch
        .retain(|slug, _| CLIAgent::from_slug(slug) != Some(agent));
    cfg.agent_launch.insert(agent.slug().to_string(), line);
}

/// The line that resumes `agent`'s session `session_id`, carrying the flags
/// `agent_launch` gives the agent: a session started with
/// `--dangerously-skip-permissions` comes back with it.
pub(crate) fn resume_line(
    agent: CLIAgent,
    session_id: &str,
    overrides: &HashMap<String, String>,
) -> Option<String> {
    let launch = agent.launch_command(overrides);
    let argv: Vec<String> = launch.split_whitespace().map(str::to_string).collect();
    agent.resume_command(session_id, Some(&argv))
}

/// The line that forks `agent`'s session `session_id` into a new one, with
/// the same flags [`resume_line`] carries. `None` for an agent that cannot
/// fork.
pub(crate) fn fork_line(
    agent: CLIAgent,
    session_id: &str,
    overrides: &HashMap<String, String>,
) -> Option<String> {
    let launch = agent.launch_command(overrides);
    let argv: Vec<String> = launch.split_whitespace().map(str::to_string).collect();
    agent.fork_command(session_id, Some(&argv))
}

const PROMPT_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// `line` with a fresh `--session-id` for Claude, so the pane knows which
/// conversation it is from the first keystroke — no hook, no transcript. A
/// line that already names a session, or starts none of its own, is left be.
fn with_minted_session(agent: CLIAgent, line: String) -> String {
    const NAMED: &[&str] = &[
        "--session-id",
        "--resume",
        "-r",
        "--continue",
        "-c",
        "--from-pr",
    ];
    let names_one = line
        .split_whitespace()
        .any(|t| NAMED.contains(&t.split('=').next().unwrap_or(t)));
    if agent != CLIAgent::Claude || names_one {
        return line;
    }
    format!("{line} --session-id {}", uuid::Uuid::new_v4())
}

/// Type `command` into the shell `slot` holds — now if it is up, or the
/// moment it lands if it is still connecting.
///
/// A shell that is up is not yet at its prompt: typed straight in, the line
/// was echoed by the tty above everything the shell prints while it starts (a
/// banner, `fastfetch`) and read only afterwards. It waits for the shell's
/// first prompt instead ([`type_at_first_prompt`]).
pub(crate) fn run_when_ready(slot: &PaneSlot, command: String, cx: &mut App) {
    match slot {
        PaneSlot::Ready(view) => type_at_first_prompt(view, command, cx),
        PaneSlot::Connecting(pending) => {
            pending.update(cx, |pending, _| pending.spawn.run_on_land = Some(command));
        }
    }
}

/// Type `line` into `view`'s shell once it reports a prompt, or when its
/// patience runs out (a shell with no integration never reports one).
pub(crate) fn type_at_first_prompt(view: &Entity<TerminalView>, line: String, cx: &mut App) {
    let patience = crate::ui::agent_resume::prompt_patience(view, cx);
    type_within(view, line, patience, cx);
}

fn type_within(view: &Entity<TerminalView>, line: String, patience: Duration, cx: &mut App) {
    let polls = patience.as_millis().div_ceil(PROMPT_POLL.as_millis());
    let view = view.downgrade();
    cx.spawn(async move |cx| {
        for _ in 0..polls {
            let ready = view
                .read_with(cx, |view, _| view.terminal.at_prompt())
                .unwrap_or(true);
            if ready {
                break;
            }
            cx.background_executor().timer(PROMPT_POLL).await;
        }
        let _ = view.read_with(cx, |view, _| view.run_command_line(&line));
    })
    .detach();
}

impl Tty7App {
    /// The agents this window's quick launch offers, most likely first.
    pub(crate) fn offered_agents(&self, cx: &App) -> Vec<CLIAgent> {
        let cfg = cx.global::<Config>();
        let candidates = match WorkspaceStore::all(cx)
            .get(self.workspace)
            .filter(|view| view.is_remote())
        {
            Some(view) => view
                .seen_agents
                .iter()
                .filter_map(|slug| CLIAgent::from_slug(slug))
                .collect(),
            None => installed_on(
                &std::env::var_os("PATH").unwrap_or_default(),
                &cfg.agent_launch,
            ),
        };
        by_frecency(candidates, &cfg.agent_frecency, unix_now())
    }

    /// Open a new shell for `agent` and start it there.
    pub(crate) fn launch_agent(
        &mut self,
        agent: CLIAgent,
        at: SpawnWhere,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let slot = match at {
            SpawnWhere::NewTab => {
                let cwd = self.tabs.get(self.active).and_then(|t| {
                    t.pane
                        .focused_or_first(window, cx)
                        .and_then(|leaf| leaf.read(cx).spawnable_cwd())
                });
                return self.launch_agent_in(agent, cwd, window, cx);
            }
            SpawnWhere::Split => {
                self.split_slot(Axis::Horizontal, Some(SpawnAs::Shell(None)), window, cx)
            }
        };
        self.start_agent_in(agent, slot, cx);
    }

    /// Open a new tab in `cwd` for `agent` and start it there.
    pub(crate) fn launch_agent_in(
        &mut self,
        agent: CLIAgent,
        cwd: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let slot = self.new_tab_slot(cwd, None, window, cx);
        self.start_agent_in(agent, slot, cx);
    }

    fn start_agent_in(&mut self, agent: CLIAgent, slot: Option<PaneSlot>, cx: &mut Context<Self>) {
        let command = with_minted_session(
            agent,
            agent.launch_command(&cx.global::<Config>().agent_launch),
        );
        // Nothing opened (the spawn failed, or this workspace cannot host a
        // shell right now), and the reason is already on screen. The command
        // goes nowhere rather than into whatever pane was focused before.
        let Some(slot) = slot else {
            log::warn!("no pane opened for {}; not launching it", agent.slug());
            return;
        };
        run_when_ready(&slot, command, cx);
        // Recency only: the run is counted when the pane reports the agent
        // actually running, so a launch and its detection are one use, not two.
        self.update_config(cx, |cfg| {
            cfg.agent_frecency
                .entry(agent.slug().to_string())
                .or_default()
                .last_used = unix_now();
        });
    }

    /// Reopen `agent`'s session `session_id` in a new tab in `cwd`, the
    /// directory it ran in — an agent keys its history by directory, and
    /// `--resume` elsewhere finds nothing. The resume carries the flags the
    /// agent is configured to launch with (`agent_launch`), so a session
    /// started with them comes back with them.
    ///
    /// With `fork`, the session is branched instead: the agent starts a new
    /// one from its history and the original is left as it was.
    pub(crate) fn resume_session(
        &mut self,
        agent: CLIAgent,
        session_id: &str,
        cwd: Option<std::path::PathBuf>,
        fork: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let launch = &cx.global::<Config>().agent_launch;
        let command = match fork {
            true => fork_line(agent, session_id, launch),
            false => resume_line(agent, session_id, launch),
        };
        let Some(command) = command else {
            window.push_notification(
                t_fmt(
                    L10nKey::AppSessionNotResumable,
                    &[("name", agent.display_name())],
                ),
                cx,
            );
            return;
        };
        // A directory that is gone cannot hold the session either; say so
        // rather than resume into a history the agent will not find. Only
        // asked of this machine: a remote session's directory is not here.
        if self.spawn_host(cx).is_local()
            && let Some(dir) = cwd.as_ref().filter(|dir| !dir.is_dir())
        {
            window.push_notification(
                t_fmt(
                    L10nKey::AppSessionDirectoryGone,
                    &[("path", &dir.display().to_string())],
                ),
                cx,
            );
            return;
        }
        let Some(slot) = self.new_tab_slot(cwd, None, window, cx) else {
            log::warn!("no pane opened to resume {} {session_id}", agent.slug());
            return;
        };
        run_when_ready(&slot, command, cx);
    }

    /// "New Agent Tab": launch the agent used most recently.
    pub(crate) fn new_agent_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let offered = self.offered_agents(cx);
        match most_recent(&offered, &cx.global::<Config>().agent_frecency) {
            Some(agent) => self.launch_agent(agent, SpawnWhere::NewTab, window, cx),
            None => {
                let remote = WorkspaceStore::remote_ref(cx, self.workspace).is_some();
                window.push_notification(
                    crate::ui::i18n::t(if remote {
                        L10nKey::AppNoAgentSeenHere
                    } else {
                        L10nKey::AppNoAgentOnPath
                    }),
                    cx,
                );
            }
        }
    }

    /// A pane in this window started running `agent`.
    pub(crate) fn note_agent_detected(&mut self, agent: CLIAgent, cx: &mut Context<Self>) {
        self.update_config(cx, |cfg| {
            let entry = cfg
                .agent_frecency
                .entry(agent.slug().to_string())
                .or_default();
            entry.count = entry.count.saturating_add(1);
            entry.last_used = unix_now();
        });
        if WorkspaceStore::remote_ref(cx, self.workspace).is_some() {
            WorkspaceStore::record_agent_seen(cx, self.workspace, agent);
        }
    }

    /// "Set Current Launch Args as Default": make the focused pane's agent
    /// launch the way this one was started.
    pub(crate) fn save_agent_launch_args(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self
            .tabs
            .get(self.active)
            .and_then(|t| t.pane.focused_or_first(window, cx))
        else {
            return;
        };
        let view = view.read(cx);
        let Some(agent) = view.agent() else {
            window.push_notification(crate::ui::i18n::t(L10nKey::AppPaneNoCodingAgent), cx);
            return;
        };
        let name = agent.display_name();
        let Some(line) = view
            .agent_session()
            .and_then(|s| s.launch_argv)
            .and_then(|argv| launch_line_to_save(agent, &argv))
        else {
            window.push_notification(
                t_fmt(L10nKey::AppAgentLaunchArgsUnknown, &[("name", name)]),
                cx,
            );
            return;
        };
        let saved = line.clone();
        self.update_config(cx, |cfg| remember_launch_line(cfg, agent, saved));
        window.push_notification(
            t_fmt(
                L10nKey::AppAgentLaunchSaved,
                &[("name", name), ("command", &line)],
            ),
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod first_prompt {
        use std::io::Write as _;

        use gpui::{TestAppContext, VisualTestContext};

        use super::super::*;
        use crate::daemon::protocol::{ClientMsg, DaemonMsg};
        use crate::daemon::transport::Stream;
        use crate::terminal::view::quiet_test_pane;

        fn pane(cx: &mut TestAppContext) -> (VisualTestContext, Entity<TerminalView>, Stream) {
            crate::core::config::pin_test_config_dir();
            cx.executor().allow_parking();
            cx.update(|cx| {
                gpui_component::init(cx);
                cx.set_global(Config::default());
            });
            let mut vcx = cx.add_empty_window().clone();
            let (view, daemon) = vcx.update(|window, cx| quiet_test_pane(1, window, cx));
            (vcx, view, daemon)
        }

        /// What was typed into the pane, if anything, after `wait` of test time.
        fn typed_after(wait: Duration, vcx: &mut VisualTestContext, daemon: &mut Stream) -> bool {
            vcx.executor().advance_clock(wait);
            vcx.run_until_parked();
            daemon
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            loop {
                match ClientMsg::read(daemon) {
                    Ok(ClientMsg::Input(bytes)) => {
                        assert_eq!(bytes, b"claude\r");
                        return true;
                    }
                    Ok(_) => continue,
                    Err(_) => return false,
                }
            }
        }

        fn report_prompt(
            view: &Entity<TerminalView>,
            vcx: &mut VisualTestContext,
            daemon: &mut Stream,
        ) {
            DaemonMsg::Prompt {
                active: true,
                at_prompt: true,
                last_exit: None,
            }
            .encode(daemon)
            .unwrap();
            daemon.flush().unwrap();
            for _ in 0..400 {
                if vcx.update(|_, cx| view.read(cx).terminal.at_prompt()) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("the prompt report never arrived");
        }

        fn type_it(view: &Entity<TerminalView>, patience: Duration, vcx: &mut VisualTestContext) {
            vcx.update(|_, cx| type_within(view, "claude".into(), patience, cx));
        }

        #[gpui::test]
        fn a_shell_at_its_prompt_gets_the_line_at_once(cx: &mut TestAppContext) {
            let (mut vcx, view, mut daemon) = pane(cx);
            report_prompt(&view, &mut vcx, &mut daemon);
            type_it(&view, Duration::from_secs(3), &mut vcx);
            assert!(typed_after(Duration::ZERO, &mut vcx, &mut daemon));
        }

        #[gpui::test]
        fn the_line_waits_for_the_first_prompt(cx: &mut TestAppContext) {
            let (mut vcx, view, mut daemon) = pane(cx);
            type_it(&view, Duration::from_secs(30), &mut vcx);
            assert!(!typed_after(Duration::from_secs(1), &mut vcx, &mut daemon));
            report_prompt(&view, &mut vcx, &mut daemon);
            assert!(typed_after(PROMPT_POLL, &mut vcx, &mut daemon));
        }

        #[gpui::test]
        fn a_shell_without_integration_gets_it_after_the_short_wait(cx: &mut TestAppContext) {
            let (mut vcx, view, mut daemon) = pane(cx);
            type_it(&view, Duration::from_secs(3), &mut vcx);
            assert!(!typed_after(
                Duration::from_millis(2900),
                &mut vcx,
                &mut daemon
            ));
            assert!(typed_after(
                Duration::from_millis(200),
                &mut vcx,
                &mut daemon
            ));
        }

        #[gpui::test]
        fn an_integrated_shell_that_never_prompts_gets_it_at_the_cap(cx: &mut TestAppContext) {
            let (mut vcx, view, mut daemon) = pane(cx);
            type_it(&view, Duration::from_secs(30), &mut vcx);
            assert!(!typed_after(Duration::from_secs(4), &mut vcx, &mut daemon));
            assert!(typed_after(Duration::from_secs(27), &mut vcx, &mut daemon));
        }
    }

    #[test]
    fn a_claude_launch_mints_the_session_it_will_resume() {
        let line = with_minted_session(CLIAgent::Claude, "claude --model opus".into());
        let argv: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        assert!(
            CLIAgent::Claude.session_id_in_argv(&argv).is_some(),
            "{line}"
        );
        for kept in ["claude --continue", "claude -r", "claude --session-id=x"] {
            assert_eq!(with_minted_session(CLIAgent::Claude, kept.into()), kept);
        }
        assert_eq!(
            with_minted_session(CLIAgent::Codex, "codex".into()),
            "codex"
        );
    }

    fn usage(pairs: &[(&str, u32, u64)]) -> HashMap<String, ProfileUsage> {
        pairs
            .iter()
            .map(|(slug, count, last_used)| {
                (
                    slug.to_string(),
                    ProfileUsage {
                        count: *count,
                        last_used: *last_used,
                    },
                )
            })
            .collect()
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 86_400;

    #[test]
    fn agents_are_offered_most_used_first() {
        let used = usage(&[
            ("codex", 3, NOW - DAY),
            ("claude", 12, NOW - 2 * DAY),
            // Used a lot, but so long ago that it has decayed below Codex.
            ("gemini", 20, NOW - 200 * DAY),
        ]);
        let offered = by_frecency(
            [
                CLIAgent::Aider,
                CLIAgent::Gemini,
                CLIAgent::Codex,
                CLIAgent::Claude,
                CLIAgent::Amp,
            ],
            &used,
            NOW,
        );
        assert_eq!(
            offered,
            vec![
                CLIAgent::Claude,
                CLIAgent::Codex,
                CLIAgent::Gemini,
                // Never used: they keep the order they were found in.
                CLIAgent::Aider,
                CLIAgent::Amp,
            ]
        );
    }

    #[test]
    fn new_agent_tab_opens_the_most_recently_used_agent() {
        let offered = [CLIAgent::Claude, CLIAgent::Codex, CLIAgent::Gemini];
        // Claude is used more, but Codex was the last one run or launched.
        let used = usage(&[
            ("claude", 40, NOW - DAY),
            ("codex", 1, NOW - 60),
            ("aider", 5, NOW),
        ]);
        assert_eq!(most_recent(&offered, &used), Some(CLIAgent::Codex));
        // Aider was more recent still, but it is not offered here.
        // Nothing used yet: the first one offered.
        assert_eq!(
            most_recent(&offered, &HashMap::new()),
            Some(CLIAgent::Claude)
        );
        assert_eq!(most_recent(&[], &used), None);
        // A launch that only stamped recency counts as the most recent.
        let launched = usage(&[("claude", 40, NOW - DAY), ("gemini", 0, NOW)]);
        assert_eq!(most_recent(&offered, &launched), Some(CLIAgent::Gemini));
    }

    #[test]
    fn saved_launch_args_survive_a_round_trip_through_the_command_line() {
        let run = argv(&[
            "/opt/homebrew/bin/claude",
            "--append-system-prompt",
            "be terse, don't guess",
            "--add-dir",
            "/Users/me/My Projects",
            "--model",
            "opus",
            "--resume",
            "abc-123",
        ]);
        let line = launch_line_to_save(CLIAgent::Claude, &run).unwrap();
        assert_eq!(
            crate::ui::app::split_shell_args(&line).unwrap(),
            argv(&[
                "claude",
                "--append-system-prompt",
                "be terse, don't guess",
                "--add-dir",
                "/Users/me/My Projects",
                "--model",
                "opus",
            ])
        );
        // And the saved line is recognised as the agent it launches.
        assert_eq!(
            CLIAgent::detect_from_command_with(&line, &HashMap::new()),
            Some(CLIAgent::Claude)
        );
    }

    #[test]
    fn set_current_launch_args_as_default_writes_the_agent_launch_entry() {
        let mut cfg = Config::default();
        cfg.agent_launch
            .insert("Claude".to_string(), "claude --old".to_string());
        cfg.agent_launch
            .insert("codex".to_string(), "codex --model o3".to_string());
        let line = launch_line_to_save(
            CLIAgent::Claude,
            &argv(&["claude", "--dangerously-skip-permissions", "--continue"]),
        )
        .unwrap();
        remember_launch_line(&mut cfg, CLIAgent::Claude, line);

        let json: serde_json::Value = serde_json::to_value(&cfg.0).unwrap();
        assert_eq!(
            json["agent_launch"],
            serde_json::json!({
                "claude": "claude --dangerously-skip-permissions",
                "codex": "codex --model o3",
            })
        );
        assert_eq!(
            CLIAgent::Claude.launch_command(&cfg.agent_launch),
            "claude --dangerously-skip-permissions"
        );
    }

    #[test]
    fn every_agent_has_a_bindable_launch_action() {
        for agent in CLIAgent::ALL {
            let name = launch_action_name(agent);
            assert_eq!(agent_for_launch_action(name), Some(agent), "{name}");
        }
        assert_eq!(agent_for_launch_action("LaunchAgent:nobody"), None);
        assert_eq!(agent_for_launch_action("NewAgentTab"), None);
    }

    #[test]
    fn only_agents_whose_launch_program_is_on_path_are_offered() {
        let dir = tempfile::TempDir::new().unwrap();
        for name in ["codex", "cc"] {
            let path = dir.path().join(name);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let path = dir.path().as_os_str();
        assert_eq!(installed_on(path, &HashMap::new()), vec![CLIAgent::Codex]);
        // With an override, the override's program is what has to be there.
        let overrides: HashMap<String, String> = [
            ("claude".to_string(), "cc --fast".to_string()),
            ("codex".to_string(), "codex-nightly".to_string()),
        ]
        .into_iter()
        .collect();
        assert_eq!(installed_on(path, &overrides), vec![CLIAgent::Claude]);
    }

    #[test]
    fn a_resume_carries_the_configured_launch_flags() {
        let none = HashMap::new();
        assert_eq!(
            resume_line(CLIAgent::Claude, "abc-1", &none).as_deref(),
            Some("claude --resume abc-1")
        );
        assert_eq!(
            resume_line(CLIAgent::Codex, "0199-x", &none).as_deref(),
            Some("codex resume 0199-x")
        );
        let flagged = HashMap::from([(
            "claude".to_string(),
            "claude --dangerously-skip-permissions".to_string(),
        )]);
        assert_eq!(
            resume_line(CLIAgent::Claude, "abc-1", &flagged).as_deref(),
            Some("claude --dangerously-skip-permissions --resume abc-1")
        );
    }

    #[test]
    fn an_id_that_could_smuggle_a_command_is_not_resumed() {
        let none = HashMap::new();
        assert_eq!(resume_line(CLIAgent::Claude, "x; rm -rf ~", &none), None);
        assert_eq!(resume_line(CLIAgent::Claude, "", &none), None);
    }
}
