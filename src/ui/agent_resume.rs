//! The fork's agent-resume features, kept out of `app.rs` so rebasing on
//! upstream touches as little of it as possible: which restored tabs sleep,
//! waking their agents (`--continue`, the palette command, and
//! `resume_agents_on_launch`, also after Restart Server), its Settings row
//! and dialog copy, and the line a restored agent pane types.

use std::collections::{HashMap, HashSet};

use gpui::{AnyElement, App, Context, Entity, Global, IntoElement as _, Window};

use crate::core::config::Config;
use crate::core::session::{SessionPane, SessionTab, WorkspaceId, WorkspaceStore};
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
        let weak = view.downgrade();
        view.update(cx, |_, cx| {
            // Detached: landing inside the view's update would lease it, and
            // `type_at_first_prompt` reads it — a panic that quit the app.
            HostOps::run_detached(
                host,
                cx,
                move |h| {
                    h.fork()
                        .resume_plan(agent, &id)
                        .unwrap_or(ResumePlan::Resume)
                },
                move |cx, plan| {
                    if let (Some(view), Some(line)) = (weak.upgrade(), resume.line(plan)) {
                        type_at_first_prompt(&view, line, cx);
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
    /// sleep because nothing of them survived, so a tab the user hibernated
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
        self.wake_agent_tabs(ids, None, window, cx);
    }

    fn asleep_layouts(&self) -> Vec<(TabId, Option<&SessionPane>)> {
        self.tabs
            .iter()
            .map(|t| (t.tree_id.get(), t.asleep_layout()))
            .collect()
    }

    /// Wake `ids` through the wake pool, a few at a time, and tell each
    /// agent `prompt` — the morning after a reboot, in one step. Tabs are
    /// found by id in this window when their turn comes: one closed or
    /// dragged to another window meanwhile counts as gone, and stays asleep.
    fn wake_agent_tabs(
        &mut self,
        ids: Vec<TabId>,
        prompt: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::ui::wake_pool::queue(ids, prompt, window.window_handle(), cx);
    }
}

/// An automatic wake (launch, Restart Server): the agent tabs restore found
/// dead, resumed without a prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Wake;

/// The tabs restore put to sleep because none of their panes survived (a
/// reboot, Quit and Stop, a restart that killed the shells), as opposed to
/// tabs the user hibernated, until a wake succeeds, the user hibernates or
/// closes the tab. Kept in `restored-asleep.json`: the tree marks every
/// sleeping tab `hibernated` alike, so the next launch could not otherwise
/// tell one tty7 put to sleep from one the user did. The file is this
/// client's: a tab another client hibernates stays in it, and this client's
/// next launch wakes it.
#[derive(Default)]
struct RestoredDead {
    ids: HashSet<TabId>,
    loaded: bool,
    flush_queued: bool,
}

impl Global for RestoredDead {}

const RESTORED_FILE: &str = "restored-asleep.json";

#[cfg(not(test))]
fn restored_file() -> Option<std::path::PathBuf> {
    crate::core::config::config_path(RESTORED_FILE)
}

/// Each test thread its own file, so tests never share one.
#[cfg(test)]
fn restored_file() -> Option<std::path::PathBuf> {
    Some(std::env::temp_dir().join(format!(
        "tty7-restored-{}-{:?}.json",
        std::process::id(),
        std::thread::current().id()
    )))
}

/// Removes this test thread's file.
#[cfg(test)]
pub(crate) fn remove_restored_file() {
    if let Some(path) = restored_file() {
        let _ = std::fs::remove_file(path);
    }
}

fn load_restored() -> HashSet<TabId> {
    let Some(text) = restored_file().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return HashSet::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        log::warn!("{RESTORED_FILE} is unreadable, starting it over: {e}");
        HashSet::new()
    })
}

/// [`RestoredDead`], read off disk the first time.
fn restored(cx: &mut App) -> &mut RestoredDead {
    let state = cx.default_global::<RestoredDead>();
    if !state.loaded {
        state.ids = load_restored();
        state.loaded = true;
    }
    state
}

/// [`RestoredDead`] after `edit`, which answers whether it changed it. A
/// change is written once the current effect cycle is over, so a restore of
/// thirty tabs writes the file once.
fn edit_restored(cx: &mut App, edit: impl FnOnce(&mut HashSet<TabId>) -> bool) {
    let state = restored(cx);
    if edit(&mut state.ids) && !state.flush_queued {
        state.flush_queued = true;
        cx.defer(flush_restored);
    }
}

fn flush_restored(cx: &mut App) {
    let state = cx.default_global::<RestoredDead>();
    state.flush_queued = false;
    let text = serde_json::to_vec(&state.ids).unwrap_or_default();
    if let Some(path) = restored_file()
        && let Err(e) = crate::core::config::write_atomic(&path, &text)
    {
        log::warn!("could not save {RESTORED_FILE}: {e}");
    }
}

/// Tab `id` woke, or the user hibernated or closed it: no automatic wake is
/// to touch it.
pub(crate) fn forget_restored(id: TabId, cx: &mut App) {
    edit_restored(cx, |set| set.remove(&id));
}

/// Workspace `ws` is being deleted: none of its tabs is any wake's.
pub(crate) fn forget_workspace_restored(ws: WorkspaceId, cx: &mut App) {
    let Some((views, _)) = crate::ui::machine_mirror::tab_views_for(cx, ws) else {
        return;
    };
    let ids: Vec<TabId> = views.iter().map(|v| v.id).collect();
    edit_restored(cx, |set| ids.iter().fold(false, |c, id| set.remove(id) | c));
}

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
        edit_restored(cx, |set| match dead {
            true => set.insert(id),
            // Asleep since an earlier restore found it dead, and not woken
            // since: still tty7's to wake, not the user's.
            false if st.hibernated => false,
            false => set.remove(&id),
        });
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

/// The tabs an automatic wake picks, [`agent_tabs`] that restore found dead.
/// Read only: a tab leaves [`RestoredDead`] when its wake succeeds, so one
/// cut short by a quit or a failure is still tty7's to wake next launch.
fn take_restored(tabs: Vec<(TabId, Option<&SessionPane>)>, cx: &mut App) -> Vec<TabId> {
    let dead = &restored(cx).ids;
    agent_tabs(tabs.iter().copied().filter(|(id, _)| dead.contains(id)))
}

/// Whether launch wakes agents: under `--continue` or
/// `resume_agents_on_launch`.
fn launch_wake(continue_flag: bool, cfg: &Config) -> Option<Wake> {
    (continue_flag || cfg.fork.resume_agents_on_launch).then_some(Wake)
}

/// This launch's agent wake, run once in each workspace a window lands tabs
/// for — the one launch opened, and any opened after it, such as the hotkey
/// window, which launch never opens.
#[derive(Default)]
struct LaunchWake {
    wake: Option<Wake>,
    done: HashSet<WorkspaceId>,
}

impl Global for LaunchWake {}

/// Launch's agent wake (`--continue`, `resume_agents_on_launch`): held for
/// every window's tabs to take as they land.
pub fn wake_launch_window(cx: &mut App, continue_flag: bool) {
    let wake = launch_wake(continue_flag, cx.global::<Config>());
    cx.set_global(LaunchWake {
        wake,
        done: HashSet::new(),
    });
    for (ws, app) in WindowRegistry::open_windows(cx) {
        let (Some(handle), Some(app)) = (WindowRegistry::window_for(cx, ws), app.upgrade()) else {
            continue;
        };
        let Some(wake) = take_launch_wake(ws, cx) else {
            continue;
        };
        let _ = handle.update(cx, |_, window, cx| {
            app.update(cx, |this, cx| this.wake_restored(wake, window, cx))
        });
    }
}

/// The launch wake for `ws`, the first time it is asked for.
pub(crate) fn take_launch_wake(ws: WorkspaceId, cx: &mut App) -> Option<Wake> {
    if !cx.has_global::<LaunchWake>() {
        return None;
    }
    let launch = cx.global_mut::<LaunchWake>();
    launch.wake.clone().filter(|_| launch.done.insert(ws))
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
            app.update(cx, |this, _| this.continue_when_tabs_land = Some(Wake));
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
    /// The Startup & Restore rows for `resume_agents_on_launch` and
    /// `agent_wake_concurrency`.
    pub(crate) fn resume_agents_settings(&self, cx: &mut Context<Self>) -> [AnyElement; 2] {
        let on = cx.global::<Config>().fork.resume_agents_on_launch;
        let resume = self.settings_switch("resume-agents", on, cx, |this, on, _, cx| {
            this.update_config(cx, |c| c.fork.resume_agents_on_launch = on)
        });
        let slider = crate::ui::wake_pool::concurrency_slider(cx);
        [
            (
                L10nKey::SettingsResumeAgents,
                L10nKey::SettingsResumeAgentsDesc,
                resume,
            ),
            (
                L10nKey::SettingsAgentWakeConcurrency,
                L10nKey::SettingsAgentWakeConcurrencyDesc,
                slider,
            ),
        ]
        .map(|(title, desc, control)| {
            self.settings_row(t(title), t(desc), control, cx)
                .into_any_element()
        })
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
    use std::io::Write as _;
    use std::time::Duration;

    use super::{
        AtPrompt, LaunchWake, RestoredDead, Resume, ResumePlan, Wake, agent_tabs, flush_restored,
        forget_restored, launch_wake, layout_has_live_pane, load_restored, record_restores_asleep,
        remove_restored_file, restart_wakes, take_launch_wake, take_restored, with_minted_session,
    };
    use crate::core::config::Config;
    use crate::core::session::SessionPane;
    use crate::daemon::protocol::{ClientMsg, DaemonMsg};
    use crate::terminal::view::quiet_test_pane;
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

    #[gpui::test]
    fn a_local_resume_types_its_line_without_panicking(cx: &mut gpui::TestAppContext) {
        crate::core::config::pin_test_config_dir();
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(Config::default());
        });
        let mut vcx = cx.add_empty_window().clone();
        let (view, mut daemon) = vcx.update(|window, cx| quiet_test_pane(1, window, cx));
        let resume = Resume {
            agent: CLIAgent::Codex,
            session_id: "th_1".into(),
            launch_argv: None,
            prompt: None,
        };
        DaemonMsg::Prompt {
            active: true,
            at_prompt: true,
            last_exit: None,
        }
        .encode(&mut daemon)
        .unwrap();
        daemon.flush().unwrap();
        while !vcx.update(|_, cx| view.read(cx).terminal.at_prompt()) {
            std::thread::sleep(Duration::from_millis(5));
        }
        // It used to land inside the view's update and panic reading it.
        vcx.update(|_, cx| AtPrompt::Resume(resume).run(&view, cx));
        daemon
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        for _ in 0..400 {
            vcx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        loop {
            match ClientMsg::read(&mut daemon) {
                Ok(ClientMsg::Input(bytes)) => {
                    assert_eq!(bytes, b"codex resume th_1\r");
                    break;
                }
                Ok(_) => continue,
                Err(e) => panic!("nothing typed: {e}"),
            }
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
                .is_some_and(|d| d.ids.contains(&id))
        };
        let recorded_on_disk = |id| load_restored().contains(&id);
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
            // The tree now marks it `hibernated` like any sleeping tab; the
            // next launch's restore still takes it for one tty7 put to sleep.
            assert!(record_restores_asleep(None, Some(&empty), &hibernated, cx));
            assert!(recorded(cx, id), "a relaunch keeps it tty7's to wake");
            flush_restored(cx);
            cx.remove_global::<RestoredDead>();
            assert!(recorded_on_disk(id), "and so does a new process");
            assert!(record_restores_asleep(None, Some(&empty), &hibernated, cx));
            assert!(recorded(cx, id));

            forget_restored(id, cx);
            assert!(record_restores_asleep(None, Some(&empty), &hibernated, cx));
            assert!(!recorded(cx, id), "hibernating it on purpose unrecords it");
            flush_restored(cx);
            assert!(!recorded_on_disk(id));

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
            assert!(recorded(cx, other), "only a wake that lands lets go of it");
            forget_restored(other, cx);
            assert!(take_restored(asleep.clone(), cx).is_empty());
            assert_eq!(agent_tabs(asleep).len(), 2, "Continue All wakes both");
        });
        remove_restored_file();
    }

    #[gpui::test]
    fn every_workspace_takes_the_launch_wake_once(cx: &mut gpui::TestAppContext) {
        use crate::core::session::WorkspaceId;
        let (launched, hotkey) = (WorkspaceId::new(), WorkspaceId::new());
        cx.update(|cx| {
            assert_eq!(take_launch_wake(launched, cx), None, "before launch");
            cx.set_global(LaunchWake {
                wake: Some(Wake),
                done: Default::default(),
            });
            assert!(take_launch_wake(launched, cx).is_some());
            assert_eq!(take_launch_wake(launched, cx), None, "only once");
            assert!(
                take_launch_wake(hotkey, cx).is_some(),
                "a window opened after launch still gets it"
            );
            assert_eq!(take_launch_wake(hotkey, cx), None);
        });
    }

    #[test]
    fn a_launch_wakes_under_resume_or_continue_only() {
        for (resume, flag, want) in [
            (true, false, Some(Wake)),
            (true, true, Some(Wake)),
            (false, true, Some(Wake)),
            (false, false, None),
        ] {
            let mut cfg = Config::default();
            cfg.fork.resume_agents_on_launch = resume;
            assert_eq!(launch_wake(flag, &cfg), want, "{resume} {flag}");
        }
        assert_eq!(
            launch_wake(false, &Config::default()),
            Some(Wake),
            "on by default"
        );
    }

    #[test]
    fn with_resume_off_restart_leaves_agent_tabs_asleep() {
        let mut cfg = Config::default();
        assert!(restart_wakes(false, &cfg));
        assert!(
            !restart_wakes(true, &cfg),
            "a handoff kills nothing to resume"
        );
        cfg.fork.resume_agents_on_launch = false;
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
