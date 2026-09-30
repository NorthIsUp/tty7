//! The fork's agent-resume features, kept out of `app.rs` so rebasing on
//! upstream touches as little of it as possible: which restored tabs sleep,
//! waking their agents (`--continue`, the palette command, and
//! `resume_agents_on_launch`), and the line a restored agent pane types.

use std::time::Duration;

use gpui::{App, Context, Entity, Global, Window};

use crate::core::config::Config;
use crate::core::session::SessionPane;
use crate::terminal::view::TerminalView;
use crate::ui::agent_launch::type_at_first_prompt;
use crate::ui::app::{Tty7App, join_shell_args};
use crate::ui::host_ops::HostOps;
use crate::ui::windows::WindowRegistry;
use tty7_core::core::claude_background::ResumePlan;
use tty7_core::core::cli_agent::CLIAgent;
use tty7_core::core::machine::TabId;
use tty7_core::daemon::pane::integrates;

/// An agent session a restored pane reopens once its shell is up.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Resume {
    pub agent: CLIAgent,
    pub session_id: String,
    pub launch_argv: Option<Vec<String>>,
    /// Sent as the resumed session's next turn, where the agent takes one.
    pub prompt: Option<String>,
}

impl Resume {
    /// The one command to type under `plan`. Attach and a fresh start take
    /// no prompt: a running session needs no nudge, and a new one has nothing
    /// to continue.
    fn line(&self, plan: ResumePlan) -> Option<String> {
        let argv = self.launch_argv.as_deref();
        let fresh = match plan {
            ResumePlan::Attach(job) => return Some(format!("claude attach {job}")),
            ResumePlan::Fresh => self.agent.start_command(&self.session_id, argv),
            ResumePlan::Resume => None,
        };
        if fresh.is_some() {
            return fresh;
        }
        let cmd = self.agent.resume_command(&self.session_id, argv)?;
        Some(
            match self
                .prompt
                .as_deref()
                .filter(|p| !p.is_empty() && self.agent.resume_takes_prompt())
            {
                Some(p) => format!("{cmd} {}", join_shell_args(&[p.to_string()])),
                None => cmd,
            },
        )
    }
}

impl Resume {
    /// For a pane that was still connecting when its wake ran: the prompt it
    /// carried here in `PendingSpawn::agent_prompt`.
    pub(crate) fn landing(self, prompt: Option<String>) -> AtPrompt {
        AtPrompt::Resume(Resume { prompt, ..self })
    }
}

/// The prompt a wake in progress sends each agent it resumes, set only for
/// the length of [`Tty7App::wake_tab_with`] (as `in_background` is).
struct WakePrompt(Option<String>);

impl Global for WakePrompt {}

/// The prompt of the wake in progress, if any.
pub(crate) fn wake_prompt(cx: &App) -> Option<String> {
    cx.try_global::<WakePrompt>().and_then(|w| w.0.clone())
}

/// What a pane types at its first prompt.
pub(crate) enum AtPrompt {
    Line(String),
    Resume(Resume),
}

impl AtPrompt {
    /// Type it into `view` at its first prompt. A resume asks the pane's host
    /// how first ([`tty7_core::host::Host::resume_plan`]), off the UI thread.
    pub(crate) fn run(self, view: &Entity<TerminalView>, cx: &mut App) {
        let resume = match self {
            AtPrompt::Line(line) => return type_at_first_prompt(view, line, cx),
            AtPrompt::Resume(resume) => resume,
        };
        let Some(host) = view.read(cx).host(cx) else {
            if let Some(line) = resume.line(ResumePlan::Resume) {
                type_at_first_prompt(view, line, cx);
            }
            return;
        };
        let (agent, id) = (resume.agent, resume.session_id.clone());
        view.update(cx, |_, cx| {
            HostOps::run(
                host,
                cx,
                move |h| h.resume_plan(agent, &id),
                move |_, plan, cx| {
                    if let Some(line) = resume.line(plan) {
                        type_at_first_prompt(&cx.entity(), line, cx);
                    }
                },
            )
        });
    }
}

/// How long a new shell may take to reach a prompt that is coming: its
/// startup files can be slow (nvm, conda).
const PROMPT_CAP: Duration = Duration::from_secs(30);
/// How long to give one that will never report a prompt: no integration.
const PROMPT_WAIT: Duration = Duration::from_secs(3);

/// How long [`type_at_first_prompt`] waits on `view`: up to [`PROMPT_CAP`]
/// when a prompt report is coming (the shell has reported already, or it is
/// a local shell the daemon gives integration), [`PROMPT_WAIT`] otherwise.
pub(crate) fn prompt_patience(view: &Entity<TerminalView>, cx: &App) -> Duration {
    let view = view.read(cx);
    let local = view.pane_route().is_local()
        && view.ssh_spec().is_none()
        && view.remote_context().is_none();
    let configured = cx
        .global::<Config>()
        .shell
        .clone()
        .map(|s| (s.program, s.args));
    match view.terminal.shell_active() || (local && integrates(view.shell_spec(), configured)) {
        true => PROMPT_CAP,
        false => PROMPT_WAIT,
    }
}

impl Tty7App {
    /// [`Self::wake_tab`], with `prompt` sent to each agent it resumes.
    pub(crate) fn wake_tab_with(
        &mut self,
        index: usize,
        prompt: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        cx.set_global(WakePrompt(prompt.map(str::to_string)));
        let woke = self.wake_tab(index, window, cx);
        cx.set_global(WakePrompt(None));
        woke
    }

    /// Continue All Agents: [`Self::wake_agent_tabs`] with `continue_prompt`.
    pub(crate) fn continue_all_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.global::<Config>().fork.continue_prompt.clone();
        self.wake_agent_tabs(Some(prompt), window, cx);
    }

    /// Wake every sleeping tab with an agent session in it, one every
    /// `continue_stagger_ms`, and tell each agent `prompt` — the morning after
    /// a reboot, in one step. Tabs are found by id when their turn comes, so
    /// closing or moving one meanwhile is harmless.
    pub(crate) fn wake_agent_tabs(
        &mut self,
        prompt: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.is_empty() {
            self.continue_when_tabs_land = Some(prompt);
            return;
        }
        let stagger =
            std::time::Duration::from_millis(cx.global::<Config>().fork.continue_stagger_ms);
        let ids = agent_tabs(
            self.tabs
                .iter()
                .map(|t| (t.tree_id.get(), t.asleep_layout())),
        );
        log::info!(
            "waking agents: {} of {} tabs asleep with an agent session",
            ids.len(),
            self.tabs.len()
        );
        cx.spawn_in(window, async move |this, cx| {
            for (n, id) in ids.into_iter().enumerate() {
                if n > 0 {
                    smol::Timer::after(stagger).await;
                }
                let woke = this.update_in(cx, |this, window, cx| {
                    if let Some(i) = this.tabs.iter().position(|t| t.tree_id.get() == id) {
                        this.wake_tab_with(i, prompt.as_deref(), window, cx);
                    }
                });
                if woke.is_err() {
                    return;
                }
            }
        })
        .detach();
    }
}

/// The tabs a wake picks: asleep, with an agent session somewhere in them.
fn agent_tabs<'a>(tabs: impl IntoIterator<Item = (TabId, Option<&'a SessionPane>)>) -> Vec<TabId> {
    tabs.into_iter()
        .filter(|(_, asleep)| asleep.is_some_and(layout_has_agent_session))
        .map(|(id, _)| id)
        .collect()
}

/// What launch wakes agents with: `--continue` sends `continue_prompt`,
/// `resume_agents_on_launch` resumes without a prompt, else nothing wakes.
/// One answer, so a launch with both wakes each tab once.
fn launch_wake(continue_flag: bool, cfg: &Config) -> Option<Option<String>> {
    if continue_flag {
        Some(Some(cfg.fork.continue_prompt.clone()))
    } else if cfg.fork.resume_agents_on_launch {
        Some(None)
    } else {
        None
    }
}

/// Launch's agent wake (`--continue`, `resume_agents_on_launch`) in the
/// window launch opened.
pub fn wake_launch_window(cx: &mut App, continue_flag: bool) {
    let Some(prompt) = launch_wake(continue_flag, cx.global::<Config>()) else {
        return;
    };
    let Some(ws) = WindowRegistry::most_recent(cx) else {
        log::warn!("launch wake: no window to wake agents in");
        return;
    };
    let (Some(handle), Some(app)) = (
        WindowRegistry::window_for(cx, ws),
        WindowRegistry::app_for(cx, ws),
    ) else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        let _ = app.update(cx, |this, cx| this.wake_agent_tabs(prompt, window, cx));
    });
}

pub(crate) fn layout_has_agent_session(pane: &SessionPane) -> bool {
    match pane {
        SessionPane::Leaf {
            agent,
            agent_session_id,
            ..
        } => agent.is_some() && agent_session_id.is_some(),
        SessionPane::Split { a, b, .. } => {
            layout_has_agent_session(a) || layout_has_agent_session(b)
        }
    }
}

pub(crate) fn layout_has_live_pane(
    pane: &SessionPane,
    alive: Option<&std::collections::HashMap<u64, Option<String>>>,
) -> bool {
    match pane {
        SessionPane::Leaf { pane_id, .. } => {
            pane_id.is_some_and(|id| alive.is_some_and(|a| a.contains_key(&id)))
        }
        SessionPane::Split { a, b, .. } => {
            layout_has_live_pane(a, alive) || layout_has_live_pane(b, alive)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Resume, ResumePlan, agent_tabs, launch_wake, layout_has_live_pane};
    use crate::core::config::Config;
    use crate::core::session::SessionPane;
    use tty7_core::core::cli_agent::CLIAgent;
    use tty7_core::core::machine::TabId;

    fn agent_leaf(session: Option<&str>) -> SessionPane {
        SessionPane::Leaf {
            cwd: None,
            pane_id: None,
            shell: None,
            ssh_spec: None,
            agent: session.map(|_| CLIAgent::Claude),
            agent_session_id: session.map(Into::into),
            agent_launch_argv: None,
        }
    }

    #[test]
    fn a_wake_picks_only_sleeping_tabs_with_an_agent_session() {
        let agent = agent_leaf(Some("s1"));
        let shell = agent_leaf(None);
        let (asleep_agent, asleep_shell, awake) = (TabId::new(), TabId::new(), TabId::new());
        let picked = agent_tabs([
            (asleep_agent, Some(&agent)),
            (asleep_shell, Some(&shell)),
            (awake, None),
        ]);
        assert_eq!(picked, vec![asleep_agent]);
    }

    #[test]
    fn launch_wakes_once_with_the_prompt_only_under_continue() {
        let mut cfg = Config::default();
        cfg.fork.continue_prompt = "go".into();
        assert!(cfg.fork.resume_agents_on_launch);
        assert_eq!(launch_wake(true, &cfg), Some(Some("go".into())));
        assert_eq!(launch_wake(false, &cfg), Some(None));
        cfg.fork.resume_agents_on_launch = false;
        assert_eq!(launch_wake(true, &cfg), Some(Some("go".into())));
        assert_eq!(launch_wake(false, &cfg), None);
    }

    #[test]
    fn a_tab_sleeps_through_restore_only_when_nothing_of_it_survived() {
        use crate::core::session::{SessionAxis, SessionPane};
        let leaf = |id: Option<u64>| SessionPane::Leaf {
            cwd: None,
            pane_id: id,
            shell: None,
            ssh_spec: None,
            agent: None,
            agent_session_id: None,
            agent_launch_argv: None,
        };
        let split = SessionPane::Split {
            axis: SessionAxis::Horizontal,
            ratio: 0.5,
            a: Box::new(leaf(Some(1))),
            b: Box::new(leaf(Some(2))),
        };
        let alive: std::collections::HashMap<u64, Option<String>> = [(2, None)].into();
        assert!(layout_has_live_pane(&split, Some(&alive)));
        assert!(!layout_has_live_pane(&leaf(Some(1)), Some(&alive)));
        assert!(!layout_has_live_pane(&leaf(None), Some(&alive)));
        assert!(!layout_has_live_pane(&split, Some(&Default::default())));
    }

    #[test]
    fn a_resume_types_exactly_one_command_for_its_plan() {
        const ID: &str = "0b5c3a5e-6d0e-4c1f-9a4b-2f7f1d9e8c11";
        let resume = Resume {
            agent: CLIAgent::Claude,
            session_id: ID.into(),
            launch_argv: Some(vec!["claude".into(), "--model".into(), "opus".into()]),
            prompt: Some("go on".into()),
        };
        assert_eq!(
            resume.line(ResumePlan::Resume).as_deref(),
            Some(format!("claude --model opus --resume {ID} 'go on'").as_str())
        );
        assert_eq!(
            resume.line(ResumePlan::Fresh).as_deref(),
            Some(format!("claude --model opus --session-id {ID}").as_str())
        );
        assert_eq!(
            resume
                .line(ResumePlan::Attach("6011098d".into()))
                .as_deref(),
            Some("claude attach 6011098d")
        );
        for plan in [ResumePlan::Resume, ResumePlan::Fresh] {
            assert!(!resume.line(plan).unwrap().contains("||"));
        }

        let codex = Resume {
            agent: CLIAgent::Codex,
            session_id: "th_1".into(),
            launch_argv: None,
            prompt: None,
        };
        assert_eq!(
            codex.line(ResumePlan::Fresh).as_deref(),
            Some("codex resume th_1"),
            "an agent that can't name a new session resumes"
        );
        let unsafe_id = Resume {
            session_id: "$(boom)".into(),
            ..resume
        };
        assert_eq!(unsafe_id.line(ResumePlan::Resume), None);
    }
}
