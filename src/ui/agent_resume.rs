//! The fork's agent-resume features, kept out of `app.rs` so rebasing on
//! upstream touches as little of it as possible: which restored tabs sleep,
//! waking their agents (`--continue`, the palette command, and
//! `resume_agents_on_launch`, also after Restart Server), its Settings row
//! and dialog copy, and the line a restored agent pane types.

use std::collections::{HashMap, HashSet};

use gpui::{AnyElement, App, Context, Entity, Global, IntoElement as _, Window};

use crate::core::config::Config;
use crate::core::session::{SessionPane, SessionTab, WorkspaceStore};
use crate::terminal::PaneWorkspace;
use crate::terminal::view::TerminalView;
use crate::ui::app::{Tty7App, join_shell_args};
use crate::ui::first_prompt::{on_this_machine, type_at_first_prompt};
use crate::ui::host_ops::HostOps;
use crate::ui::i18n::{L10nKey, t};
use crate::ui::windows::WindowRegistry;
use tty7_core::core::claude_background::ResumePlan;
use tty7_core::core::cli_agent::CLIAgent;
use tty7_core::core::machine::TabId;
use tty7_core::host::HostId;

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
        match plan {
            ResumePlan::Attach(job) => Some(format!("claude attach {job}")),
            ResumePlan::Fresh => self
                .agent
                .start_command(&self.session_id, argv)
                .or_else(|| self.resumed(argv)),
            ResumePlan::Resume => self.resumed(argv),
        }
    }

    fn resumed(&self, argv: Option<&[String]>) -> Option<String> {
        let cmd = self.agent.resume_command(&self.session_id, argv)?;
        let prompt = self.prompt.as_deref();
        Some(
            match prompt.filter(|p| !p.is_empty() && self.agent.resume_takes_prompt()) {
                Some(p) => format!("{cmd} {}", join_shell_args(&[p.to_string()])),
                None => cmd,
            },
        )
    }
}

impl Resume {
    /// What a restored pane that ran `agent` reopens: nothing when session
    /// restore is off, no session id was captured, or the agent cannot resume
    /// it (an id unsafe to type).
    pub(crate) fn restored(
        agent: &Option<CLIAgent>,
        session_id: Option<&str>,
        launch_argv: Option<&[String]>,
        cx: &App,
    ) -> Option<Resume> {
        if !cx.global::<Config>().restore_agent_sessions {
            return None;
        }
        let agent = agent.as_ref()?;
        let Some(session_id) = session_id else {
            log::info!(
                "{}'s pane had no captured session id; it comes back as a plain shell",
                agent.display_name()
            );
            return None;
        };
        agent.resume_command(session_id, launch_argv)?;
        Some(Resume {
            agent: *agent,
            session_id: session_id.to_string(),
            launch_argv: launch_argv.map(<[String]>::to_vec),
            prompt: wake_prompt(cx),
        })
    }

    /// For a pane that was still connecting when its wake ran: the prompt it
    /// carried here in `PendingSpawn::agent_prompt`.
    pub(crate) fn landing(self, prompt: Option<String>) -> AtPrompt {
        AtPrompt::Resume(Resume { prompt, ..self })
    }
}

/// `line` with a fresh `--session-id` for Claude, so the pane knows which
/// conversation it is from the first keystroke — no hook, no transcript. A
/// line that already names a session, or starts none of its own, is left be.
pub(crate) fn with_minted_session(agent: CLIAgent, line: String) -> String {
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
    /// how first ([`tty7_core::core::fork_host::ForkHost::resume_plan`]), off the UI thread.
    pub(crate) fn run(self, view: &Entity<TerminalView>, cx: &mut App) {
        let resume = match self {
            AtPrompt::Line(line) => return type_at_first_prompt(view, line, cx),
            AtPrompt::Resume(resume) => resume,
        };
        // A session on another machine: a plan read from this one's files
        // would be wrong about it.
        let host = view
            .read(cx)
            .host(cx)
            .filter(|_| on_this_machine(view.read(cx)));
        let Some(host) = host else {
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
                move |h| {
                    h.fork()
                        .resume_plan(agent, &id)
                        .unwrap_or(ResumePlan::Resume)
                },
                move |_, plan, cx| {
                    if let Some(line) = resume.line(plan) {
                        type_at_first_prompt(&cx.entity(), line, cx);
                    }
                },
            )
        });
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
        let outer = cx.try_global::<WakePrompt>().and_then(|w| w.0.clone());
        debug_assert!(outer.is_none(), "a wake inside a wake");
        cx.set_global(WakePrompt(prompt.map(str::to_string)));
        let woke = self.wake_tab(index, window, cx);
        cx.set_global(WakePrompt(outer));
        woke
    }

    /// Continue All Agents: wake every sleeping agent tab, hibernated on
    /// purpose or not, with `continue_prompt`.
    pub(crate) fn continue_all_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.global::<Config>().fork.continue_prompt.clone();
        let ids = agent_tabs(self.asleep_layouts());
        log::info!("waking {} agent tab(s) of {}", ids.len(), self.tabs.len());
        self.wake_agent_tabs(ids, Some(prompt), window, cx);
    }

    /// Launch's and Restart Server's wake: only the agent tabs restore put to
    /// sleep because nothing of them survived. A tab the user hibernated
    /// stays asleep.
    pub(crate) fn wake_restored(
        &mut self,
        wake: Wake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.is_empty() {
            self.continue_when_tabs_land = Some(wake);
            return;
        }
        let ids = take_restored(self.asleep_layouts(), cx);
        log::info!(
            "waking {} restored agent tab(s) of {}",
            ids.len(),
            self.tabs.len()
        );
        self.wake_agent_tabs(ids, wake.prompt, window, cx);
    }

    fn asleep_layouts(&self) -> Vec<(TabId, Option<&SessionPane>)> {
        self.tabs
            .iter()
            .map(|t| (t.tree_id.get(), t.asleep_layout()))
            .collect()
    }

    /// Wake `ids`, one every `continue_stagger_ms`, and tell each agent
    /// `prompt` — the morning after a reboot, in one step. Tabs are found by
    /// id when their turn comes, so closing or moving one meanwhile is
    /// harmless.
    fn wake_agent_tabs(
        &mut self,
        ids: Vec<TabId>,
        prompt: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stagger =
            std::time::Duration::from_millis(cx.global::<Config>().fork.continue_stagger_ms);
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

/// An automatic wake (launch, Restart Server): what it sends each agent it
/// resumes (`None`: resume without a prompt).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Wake {
    pub prompt: Option<String>,
}

/// The tabs the last restore put to sleep because none of their panes
/// survived (a reboot, Quit and Stop, a restart that killed the shells), as
/// opposed to tabs the user hibernated. Each restore re-decides its tabs, and
/// a wake drains its window's.
#[derive(Default)]
struct RestoredDead(HashSet<TabId>);

impl Global for RestoredDead {}

/// Whether restore brings `st` back asleep: hibernated, or (with
/// `restore_asleep`) a local tab none of whose panes survived — twelve agents
/// come back as twelve places to click, not twelve cold starts racing each
/// other at launch. Records which of the two it was for [`Tty7App::wake_restored`], so the
/// call must run for every tab, not behind a short-circuit.
pub(crate) fn record_restores_asleep(
    workspace: Option<&PaneWorkspace>,
    alive: Option<&HashMap<u64, Option<String>>>,
    st: &SessionTab,
    cx: &mut App,
) -> bool {
    // A remote listing is not asked for, so only locally can "nothing of it
    // is running" be told apart from "nobody checked".
    let lazy = cx.global::<Config>().fork.restore_asleep && workspace.is_none();
    let dead = restored_dead(st, alive.filter(|_| lazy));
    if let Some(id) = st.tree_id {
        let set = &mut cx.default_global::<RestoredDead>().0;
        match dead {
            true => set.insert(id),
            false => set.remove(&id),
        };
    }
    st.hibernated || dead
}

/// Asleep because nothing of it survived, not because the user put it there.
/// `alive: None` is "nobody checked", which proves nothing dead.
fn restored_dead(st: &SessionTab, alive: Option<&HashMap<u64, Option<String>>>) -> bool {
    !st.hibernated && alive.is_some_and(|a| !layout_has_live_pane(&st.pane, Some(a)))
}

/// The tabs a wake picks: asleep, with an agent session somewhere in them.
fn agent_tabs<'a>(tabs: impl IntoIterator<Item = (TabId, Option<&'a SessionPane>)>) -> Vec<TabId> {
    tabs.into_iter()
        .filter(|(_, asleep)| asleep.is_some_and(layout_has_agent_session))
        .map(|(id, _)| id)
        .collect()
}

/// The tabs an automatic wake picks, [`agent_tabs`] that restore found dead,
/// draining every one of `tabs` from [`RestoredDead`] so the next wake waits
/// on the next restore.
fn take_restored(tabs: Vec<(TabId, Option<&SessionPane>)>, cx: &mut App) -> Vec<TabId> {
    let dead = &mut cx.default_global::<RestoredDead>().0;
    let ids = agent_tabs(tabs.iter().copied().filter(|(id, _)| dead.contains(id)));
    for (id, _) in &tabs {
        dead.remove(id);
    }
    ids
}

/// What launch wakes agents with: `--continue` sends `continue_prompt`,
/// `resume_agents_on_launch` resumes without a prompt, else nothing wakes.
/// One answer, so a launch with both wakes each tab once.
fn launch_wake(continue_flag: bool, cfg: &Config) -> Option<Wake> {
    if continue_flag {
        Some(Wake {
            prompt: Some(cfg.fork.continue_prompt.clone()),
        })
    } else if cfg.fork.resume_agents_on_launch {
        Some(Wake { prompt: None })
    } else {
        None
    }
}

/// Launch's agent wake (`--continue`, `resume_agents_on_launch`) in the
/// window launch opened.
pub fn wake_launch_window(cx: &mut App, continue_flag: bool) {
    let Some(wake) = launch_wake(continue_flag, cx.global::<Config>()) else {
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
        let _ = app.update(cx, |this, cx| this.wake_restored(wake, window, cx));
    });
}

/// Restart Server's agent wake. A restart that kills the shells brings every
/// local window's tabs back asleep, so with `resume_agents_on_launch` each
/// window wakes the agent tabs it killed once the rebuild lands them, as a
/// launch would.
/// An in-place handoff kills nothing and wakes nothing.
pub(crate) fn arm_restart_wake(in_place: bool, cx: &mut App) {
    if !restart_wakes(in_place, cx.global::<Config>()) {
        return;
    }
    for (ws, app) in WindowRegistry::open_windows(cx) {
        if WorkspaceStore::host_of(cx, ws) != HostId::LOCAL {
            continue;
        }
        if let Some(app) = app.upgrade() {
            app.update(cx, |this, _| {
                this.continue_when_tabs_land = Some(Wake { prompt: None })
            });
        }
    }
}

fn restart_wakes(in_place: bool, cfg: &Config) -> bool {
    !in_place && cfg.fork.resume_agents_on_launch
}

/// Quit and Stop's body: whether agents come back on their own next launch.
pub(crate) fn quit_stop_body(cx: &App) -> L10nKey {
    match cx.global::<Config>().fork.resume_agents_on_launch {
        true => L10nKey::QuitStopServerBodyResume,
        false => L10nKey::QuitStopServerBody,
    }
}

/// Restart Server's body where the shells die: whether agents resume after.
pub(crate) fn restart_body(cx: &App) -> L10nKey {
    match cx.global::<Config>().fork.resume_agents_on_launch {
        true => L10nKey::AppRestartServerBodyResume,
        false => L10nKey::AppRestartServerBody,
    }
}

impl Tty7App {
    /// The Startup & Restore row for `resume_agents_on_launch`.
    pub(crate) fn resume_agents_setting(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = cx.global::<Config>().fork.resume_agents_on_launch;
        let switch = self.settings_switch("resume-agents", on, cx, |this, on, _, cx| {
            this.update_config(cx, |c| c.fork.resume_agents_on_launch = on)
        });
        self.settings_row(
            t(L10nKey::SettingsResumeAgents),
            t(L10nKey::SettingsResumeAgentsDesc),
            switch,
            cx,
        )
        .into_any_element()
    }
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
    use super::{
        RestoredDead, Resume, ResumePlan, Wake, agent_tabs, launch_wake, layout_has_live_pane,
        record_restores_asleep, restart_wakes, take_restored, with_minted_session,
    };
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

    #[gpui::test]
    fn an_automatic_wake_leaves_hibernated_tabs_asleep(cx: &mut gpui::TestAppContext) {
        use crate::core::session::{RemoteTarget, SessionTab, WorkspaceId};
        use crate::terminal::PaneWorkspace;
        let tab = |hibernated| SessionTab {
            name: None,
            pane: agent_leaf(Some("s1")),
            group: None,
            last_auto: None,
            tree_id: Some(TabId::new()),
            hibernated,
            asleep_view: None,
        };
        let recorded = |cx: &mut gpui::App, id| {
            cx.try_global::<RestoredDead>()
                .is_some_and(|d| d.0.contains(&id))
        };
        let remote = PaneWorkspace {
            workspace: WorkspaceId::new(),
            target: RemoteTarget::Direct {
                user: "me".into(),
                host: "build-box".into(),
                port: 22,
            },
            spec: None,
            label: None,
            resize_echo: false,
            size_lease: false,
        };
        let empty = Default::default();
        cx.update(|cx| {
            let mut cfg = Config::default();
            cfg.fork.restore_asleep = true;
            cx.set_global(cfg);

            let (dead, other_dead) = (tab(false), tab(false));
            let id = dead.tree_id.unwrap();
            assert!(record_restores_asleep(None, Some(&empty), &dead, cx));
            assert!(recorded(cx, id));

            let hibernated = SessionTab {
                hibernated: true,
                ..dead.clone()
            };
            assert!(record_restores_asleep(None, Some(&empty), &hibernated, cx));
            assert!(!recorded(cx, id), "hibernating it on purpose unrecords it");

            let unchecked = tab(false);
            assert!(!record_restores_asleep(None, None, &unchecked, cx));
            assert!(!recorded(cx, unchecked.tree_id.unwrap()));

            let far = tab(false);
            assert!(!record_restores_asleep(
                Some(&remote),
                Some(&empty),
                &far,
                cx
            ));
            assert!(!recorded(cx, far.tree_id.unwrap()));

            // One hibernated, two dead: the window holds `hibernated` and
            // `other_dead`, both asleep.
            assert!(record_restores_asleep(None, Some(&empty), &other_dead, cx));
            let other = other_dead.tree_id.unwrap();
            let asleep = vec![
                (id, Some(&hibernated.pane)),
                (other, Some(&other_dead.pane)),
            ];
            assert_eq!(take_restored(asleep.clone(), cx), vec![other]);
            assert!(!recorded(cx, other), "a wake drains what it saw");
            assert!(take_restored(asleep.clone(), cx).is_empty());
            assert_eq!(agent_tabs(asleep).len(), 2, "Continue All wakes both");
        });
    }

    #[test]
    fn launch_wakes_once_with_the_prompt_only_under_continue() {
        let mut cfg = Config::default();
        cfg.fork.continue_prompt = "go".into();
        assert!(cfg.fork.resume_agents_on_launch);
        let go = Some(Wake {
            prompt: Some("go".into()),
        });
        assert_eq!(launch_wake(true, &cfg), go);
        assert_eq!(launch_wake(false, &cfg), Some(Wake { prompt: None }));
        cfg.fork.resume_agents_on_launch = false;
        assert_eq!(launch_wake(true, &cfg), go);
        assert_eq!(launch_wake(false, &cfg), None);
    }

    #[test]
    fn with_resume_off_launch_and_restart_leave_agent_tabs_asleep() {
        let mut cfg = Config::default();
        assert_eq!(launch_wake(false, &cfg), Some(Wake { prompt: None }));
        assert!(restart_wakes(false, &cfg));
        assert!(
            !restart_wakes(true, &cfg),
            "a handoff kills nothing to resume"
        );
        cfg.fork.resume_agents_on_launch = false;
        assert_eq!(launch_wake(false, &cfg), None);
        assert!(!restart_wakes(false, &cfg));
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
