//! The fork's agent-resume features, kept out of `app.rs` so rebasing on
//! upstream touches as little of it as possible: which restored tabs sleep,
//! waking their agents (`--continue`, the palette command, and
//! `resume_agents_on_launch`, also after Restart Server), its Settings row
//! and dialog copy, and the line a restored agent pane types.

use std::collections::{HashMap, HashSet};

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EntityId, Global, IntoElement as _,
    Subscription, Window,
};
use gpui_component::input::{InputEvent, InputState};

use crate::core::config::Config;
use crate::core::session::{SessionPane, SessionTab, WorkspaceId, WorkspaceStore};
use crate::daemon::protocol::ShellSpec;
use crate::terminal::view::{ShellParts, TerminalView};
use crate::terminal::{PaneRoute, PaneWorkspace};
use crate::ui::app::{Tab, Tty7App};
use crate::ui::first_prompt::type_at_first_prompt;
use crate::ui::i18n::{L10nKey, t};
use crate::ui::pending_pane::PendingSpawn;
use crate::ui::wake_pool::Job;
use crate::ui::windows::WindowRegistry;
use tty7_core::core::agent_history::Roots;
use tty7_core::core::claude_background::{self, ResumePlan};
use tty7_core::core::cli_agent::CLIAgent;
use tty7_core::core::fork_config::AgentResumeMode;
use tty7_core::core::last_session;
use tty7_core::core::machine::{AgentFacts, TabId};
use tty7_core::core::shell_quote::{quote_for_shell, runs_once};
use tty7_core::core::shells::login_shell;
use tty7_core::daemon::protocol::FEATURE_RUN_ONCE;
use tty7_core::daemon::spawn::local_daemon_supports;
use tty7_core::host::HostId;

/// How a restored agent pane comes back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Reopen {
    /// The session its agent reported.
    Session {
        session_id: String,
        /// Sent as the resumed session's next turn whatever its transcript
        /// says (Continue All Agents).
        prompt: Option<String>,
        /// Sent instead when the transcript shows the turn was cut off:
        /// `continue_prompt`, under `continue_interrupted_agents`.
        on_interrupt: Option<String>,
    },
    /// The line that starts the agent anew, for a pane whose agent never
    /// reported a session. A wake's prompt is dropped: there is nothing to
    /// continue.
    Launch(String),
}

/// An agent a restored pane reopens.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Resume {
    pub agent: CLIAgent,
    pub launch_argv: Option<Vec<String>>,
    pub reopen: Reopen,
}

impl Resume {
    /// The command to run under `plan`, quoted for `shell`. Attach and a
    /// fresh start take no prompt: a running session needs no nudge, and a
    /// new one has nothing to continue.
    fn line(&self, plan: ResumePlan, shell: &str) -> Option<String> {
        let (session_id, prompt, on_interrupt) = match &self.reopen {
            Reopen::Launch(line) => return Some(line.clone()),
            Reopen::Session {
                session_id,
                prompt,
                on_interrupt,
            } => (session_id, prompt.as_deref(), on_interrupt.as_deref()),
        };
        let argv = self.launch_argv.as_deref();
        let resumed = |prompt| self.resumed(session_id, argv, prompt, shell);
        match plan {
            ResumePlan::Attach(job) => Some(format!("claude attach {job}")),
            ResumePlan::Fresh => self
                .agent
                .start_command(session_id, argv)
                .or_else(|| resumed(None)),
            ResumePlan::Resume => resumed(prompt),
            ResumePlan::Closed => None,
            ResumePlan::Interrupted => resumed(prompt.or(on_interrupt)),
        }
    }

    fn resumed(
        &self,
        session_id: &str,
        argv: Option<&[String]>,
        prompt: Option<&str>,
        shell: &str,
    ) -> Option<String> {
        let cmd = self.agent.resume_command(session_id, argv)?;
        Some(
            match prompt.filter(|p| !p.is_empty() && self.agent.resume_takes_prompt()) {
                Some(p) => format!("{cmd} {}", quote_for_shell(p, Some(shell))),
                None => cmd,
            },
        )
    }

    /// What a restored pane that ran `agent` reopens: nothing when session
    /// restore is off or the agent cannot resume its session (an id unsafe
    /// to run); the agent started anew when no session id was captured.
    /// `prompt` is the wake's.
    pub(crate) fn restored(
        agent: &Option<CLIAgent>,
        session_id: Option<&str>,
        launch_argv: Option<&[String]>,
        prompt: Option<String>,
        cx: &App,
    ) -> Option<Resume> {
        let cfg = cx.global::<Config>();
        if !cfg.restore_agent_sessions {
            return None;
        }
        let agent = agent.as_ref()?;
        // A record an older daemon wrote has the id only in the argv it saved.
        let named = launch_argv.and_then(|argv| agent.session_id_in_argv(argv));
        let reopen = match session_id.or(named.as_deref()) {
            // Codex starts its session's hooks on the first turn, so an agent
            // nobody typed into yet has none to resume.
            None => {
                log::info!(
                    "{}'s pane had no captured session id; it starts anew",
                    agent.display_name()
                );
                let line = agent.relaunch_command(launch_argv, &cfg.agent_launch);
                Reopen::Launch(with_minted_session(*agent, line))
            }
            Some(session_id) => {
                agent.resume_command(session_id, launch_argv)?;
                let on_interrupt = cfg
                    .fork
                    .continue_interrupted_agents
                    .then(|| cfg.fork.continue_prompt.clone());
                Reopen::Session {
                    session_id: session_id.to_string(),
                    prompt,
                    on_interrupt: on_interrupt.filter(|p| !p.is_empty()),
                }
            }
        };
        Some(Resume {
            agent: *agent,
            launch_argv: launch_argv.map(<[String]>::to_vec),
            reopen,
        })
    }

    /// How to reopen it. Reads Claude's files, so off the UI thread; `local`
    /// is whether they are on this machine.
    fn plan(&self, local: bool) -> ResumePlan {
        let session_id = match &self.reopen {
            Reopen::Launch(_) => return ResumePlan::Fresh,
            Reopen::Session { session_id, .. } => session_id,
        };
        let plan = match local {
            true => Roots::local().map_or(ResumePlan::Resume, |r| {
                claude_background::resume_plan(&r.claude, self.agent, session_id)
            }),
            false => ResumePlan::Resume,
        };
        log::info!("resuming {session_id} as {plan:?}");
        plan
    }
}

/// A pending pane's resume and how it is delivered, decided once, at the
/// pane's first spawn, so its landing does the same whatever the settings say
/// by then: never twice, never not at all.
#[derive(Clone)]
pub(crate) struct PaneResume {
    resume: Resume,
    /// The shell the pane runs, for quoting.
    shell: String,
    /// Its session's files are on this machine.
    local: bool,
    /// The shell runs it once as the tab's program; else it is typed.
    run_once: bool,
}

impl PaneResume {
    fn of(spawn: &PendingSpawn, daemon_runs_once: bool, cx: &App) -> Option<PaneResume> {
        let resume = Resume::restored(
            &spawn.agent,
            spawn.agent_session_id.as_deref(),
            spawn.agent_launch_argv.as_deref(),
            spawn.agent_prompt.clone(),
            cx,
        )?;
        let route = PaneRoute::for_workspace(spawn.workspace.as_ref());
        let mut pane = PaneResume::typed(resume, spawn.shell.as_ref(), &route, cx);
        let mode = cx.global::<Config>().fork.agent_resume_mode;
        pane.run_once =
            daemon_runs_once && runs_as_program(mode, pane.local, spawn.restore_pane, &pane.shell);
        Some(pane)
    }

    /// `resume`, typed into a pane running `shell` (else the configured
    /// shell, else the login one) over `route`.
    fn typed(resume: Resume, shell: Option<&ShellSpec>, route: &PaneRoute, cx: &App) -> PaneResume {
        let shell = shell
            .map(|s| s.program.clone())
            .or_else(|| {
                cx.global::<Config>()
                    .shell
                    .as_ref()
                    .map(|s| s.program.clone())
            })
            .unwrap_or_else(login_shell);
        PaneResume {
            // `first_prompt`'s "on this machine": a local route and not WSL,
            // whose sessions keep their files on the other side.
            local: route.is_local() && !is_wsl(&shell),
            resume,
            shell,
            run_once: false,
        }
    }

    /// Off the UI thread.
    fn line(&self) -> Option<String> {
        self.resume.line(self.resume.plan(self.local), &self.shell)
    }
}

fn is_wsl(shell: &str) -> bool {
    let base = std::path::Path::new(shell)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(shell);
    base.eq_ignore_ascii_case("wsl")
}

/// Whether a resume runs as the tab's program: asked for, on this machine
/// (a remote daemon may not know the request), replacing a known pane (the
/// request rides its restore), in a shell that can run a command once.
fn runs_as_program(mode: AgentResumeMode, local: bool, restore: Option<u64>, shell: &str) -> bool {
    mode == AgentResumeMode::Program && local && restore.is_some() && runs_once(shell)
}

/// Decide `spawn`'s resume, once; a retry keeps the first answer.
fn decide(spawn: &mut PendingSpawn, daemon_runs_once: bool, cx: &App) {
    if spawn.resume.is_none() {
        spawn.resume = PaneResume::of(spawn, daemon_runs_once, cx);
    }
}

/// A pending pane's spawn, to run off the UI thread; a resume that runs as
/// the tab's program rides along for the shell to run once.
pub(crate) fn spawn_job(
    spawn: &mut PendingSpawn,
    cx: &App,
) -> impl FnOnce() -> Result<ShellParts, String> + Send + use<> {
    decide(spawn, local_daemon_supports(FEATURE_RUN_ONCE), cx);
    let spawn = spawn.clone();
    let resume = spawn.resume.clone().filter(|r| r.run_once);
    move || {
        TerminalView::spawn_shell_terminal_in(
            spawn.workspace,
            spawn.working_directory,
            spawn.restore_pane,
            spawn.shell,
            spawn.owner,
            spawn.grid,
            resume.and_then(|r| r.line()),
        )
        .map_err(|e| format!("{e:#}"))
    }
}

/// The resume a landed pane types: the one its spawn decided to type.
fn typed(spawn: &PendingSpawn) -> Option<PaneResume> {
    spawn.resume.clone().filter(|r| !r.run_once)
}

/// A pending pane landed in `view`: type its quick launch's line, or its
/// resume when that is typed. Nothing for a pane that reattached.
pub(crate) fn landed(
    view: &Entity<TerminalView>,
    spawn: &PendingSpawn,
    restored: bool,
    cx: &mut App,
) {
    if restored {
        return;
    }
    if let Some(line) = spawn.run_on_land.clone() {
        return type_at_first_prompt(view, line, cx);
    }
    if let Some(resume) = typed(spawn) {
        type_resume(view, resume, cx);
    }
}

/// Type `resume`'s line into `view` at its prompt, once its plan is read.
fn type_resume(view: &Entity<TerminalView>, resume: PaneResume, cx: &mut App) {
    let view = view.clone();
    // Typed once the plan lands, outside every update: typing reads the view,
    // which panics inside its own update (#67).
    cx.spawn(async move |cx| {
        let line = cx
            .background_executor()
            .spawn(async move { resume.line() })
            .await;
        if let Some(line) = line {
            cx.update(|cx| type_at_first_prompt(&view, line, cx));
        }
    })
    .detach();
}

/// The session `view`'s record kept (`last_session`) after its agent died,
/// told `prompt`; `None` while an agent or a command holds it. An idle shell
/// whose agent left no session is the user's, so its agent is not started anew.
fn dead_session(view: &TerminalView, prompt: Option<&str>, cx: &App) -> Option<Resume> {
    if view.agent().is_some() || !view.terminal.at_prompt() {
        return None;
    }
    let (agent, session_id, argv) = sleeping_agent(view, cx);
    Resume::restored(
        &agent,
        session_id.as_deref(),
        argv.as_deref(),
        prompt.map(str::to_string),
        cx,
    )
    .filter(|r| matches!(r.reopen, Reopen::Session { .. }))
}

/// `tab`'s panes whose agent died back to an idle shell.
fn dead_panes(tab: &Tab, cx: &App) -> Vec<EntityId> {
    tab.pane
        .terminals()
        .into_iter()
        .filter(|view| dead_session(view.read(cx), None, cx).is_some())
        .map(|view| view.entity_id())
        .collect()
}

/// [`dead_session`]'s resume, typed into `view`.
fn dead_agent(view: &TerminalView, prompt: Option<&str>, cx: &App) -> Option<PaneResume> {
    let resume = dead_session(view, prompt, cx)?;
    let shell = view.shell_spec();
    Some(PaneResume::typed(
        resume,
        shell.as_ref(),
        &view.pane_route(),
        cx,
    ))
}

/// The pending spawn a restored leaf takes when it held an agent that is no
/// longer running: its resume is planned off the UI thread. `None` for any
/// other leaf, which restores the plain way.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dead_agent_spawn(
    leaf: &SessionPane,
    workspace: Option<&PaneWorkspace>,
    owner: WorkspaceId,
    cwd: &Option<std::path::PathBuf>,
    restore: Option<u64>,
    alive: Option<&HashMap<u64, Option<String>>>,
    font_size: f32,
    grid: Option<crate::ui::background_grid::Grid>,
    cx: &App,
) -> Option<PendingSpawn> {
    let SessionPane::Leaf {
        shell,
        agent,
        agent_session_id,
        agent_launch_argv,
        ..
    } = leaf
    else {
        return None;
    };
    let running = restore.is_some_and(|id| alive.is_some_and(|a| a.contains_key(&id)));
    if running {
        return None;
    }
    Resume::restored(
        agent,
        agent_session_id.as_deref(),
        agent_launch_argv.as_deref(),
        None,
        cx,
    )?;
    Some(PendingSpawn {
        workspace: workspace.cloned(),
        working_directory: cwd.clone(),
        restore_pane: restore,
        shell: shell.clone(),
        agent: *agent,
        agent_session_id: agent_session_id.clone(),
        agent_launch_argv: agent_launch_argv.clone(),
        agent_prompt: wake_prompt(cx),
        owner: Some(owner),
        font_size,
        grid,
        ..Default::default()
    })
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
/// the length of [`Tty7App::start_job`]'s wake (as `in_background` is).
struct WakePrompt(Option<String>);

impl Global for WakePrompt {}

/// The prompt of the wake in progress, if any.
pub(crate) fn wake_prompt(cx: &App) -> Option<String> {
    cx.try_global::<WakePrompt>().and_then(|w| w.0.clone())
}

impl Tty7App {
    /// Start tab `index`'s wake pool `job`, with `prompt` sent to each agent
    /// it resumes; false when nothing could be started.
    pub(crate) fn start_job(
        &mut self,
        index: usize,
        job: &Job,
        prompt: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Job::ResumeDead(panes) = job else {
            return self.wake_tab_with(index, prompt, window, cx);
        };
        let mut resumed = false;
        for view in self.tabs[index].pane.terminals() {
            if !panes.contains(&view.entity_id()) {
                continue;
            }
            if let Some(resume) = dead_agent(view.read(cx), prompt, cx) {
                type_resume(&view, resume, cx);
                resumed = true;
            }
        }
        resumed
    }

    /// [`Self::wake_tab`], with `prompt` sent to each agent it resumes.
    fn wake_tab_with(
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
    /// purpose or not, and resume every agent that died back to its shell
    /// (a crash, its runtime killed), with `continue_prompt`.
    pub(crate) fn continue_all_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.global::<Config>().fork.continue_prompt.clone();
        let mut jobs = wakes(agent_tabs(self.asleep_layouts()));
        let asleep = jobs.len();
        for tab in self.tabs.iter().filter(|t| !t.is_asleep()) {
            let dead = dead_panes(tab, cx);
            if !dead.is_empty() {
                jobs.push((tab.tree_id.get(), Job::ResumeDead(dead)));
            }
        }
        log::info!(
            "waking {asleep} agent tab(s), resuming {} dead, of {}",
            jobs.len() - asleep,
            self.tabs.len()
        );
        self.wake_agent_tabs(jobs, Some(prompt), window, cx);
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
        self.wake_agent_tabs(wakes(ids), None, window, cx);
    }

    fn asleep_layouts(&self) -> Vec<(TabId, Option<&SessionPane>)> {
        self.tabs
            .iter()
            .map(|t| (t.tree_id.get(), t.asleep_layout()))
            .collect()
    }

    /// Start `jobs` through the wake pool, a few at a time, and tell each
    /// agent `prompt` — the morning after a reboot, in one step. Tabs are
    /// found by id in this window when their turn comes: one closed or
    /// dragged to another window meanwhile counts as gone, and stays asleep.
    fn wake_agent_tabs(
        &mut self,
        jobs: Vec<(TabId, Job)>,
        prompt: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::ui::wake_pool::queue(jobs, prompt, window.window_handle(), cx);
    }
}

fn wakes(ids: Vec<TabId>) -> Vec<(TabId, Job)> {
    ids.into_iter().map(|id| (id, Job::Wake)).collect()
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
        .filter(|(_, asleep)| asleep.is_some_and(layout_has_agent))
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
        false => L10nKey::QuitStopServerBodyAsleep,
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
    /// The Startup & Restore rows: Resume agents on workspace restart,
    /// Agents starting at once, How agents resume, Continue interrupted
    /// agents and the Continue message.
    pub(crate) fn resume_agents_settings(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let fork = cx.global::<Config>().fork.clone();
        let resume = self.settings_switch(
            "resume-agents",
            fork.resume_agents_on_launch,
            cx,
            |this, on, _, cx| this.update_config(cx, |c| c.fork.resume_agents_on_launch = on),
        );
        let slider = crate::ui::wake_pool::concurrency_slider(cx);
        let modes = [AgentResumeMode::Program, AgentResumeMode::Typed];
        let labels = [
            t(L10nKey::SettingsAgentResumeProgram),
            t(L10nKey::SettingsAgentResumeTyped),
        ];
        let mode = self.settings_choice(
            "agent-resume-mode",
            &labels.each_ref().map(|l| l.as_ref()),
            modes
                .iter()
                .position(|m| *m == fork.agent_resume_mode)
                .unwrap_or(1),
            cx,
            move |this, ix, _, cx| {
                let mode = modes.get(ix).copied().unwrap_or_default();
                this.update_config(cx, |c| c.fork.agent_resume_mode = mode)
            },
        );
        let interrupted = self.settings_switch(
            "continue-interrupted-agents",
            fork.continue_interrupted_agents,
            cx,
            |this, on, _, cx| this.update_config(cx, |c| c.fork.continue_interrupted_agents = on),
        );
        let mut rows = vec![
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
            (
                L10nKey::SettingsAgentResumeMode,
                L10nKey::SettingsAgentResumeModeDesc,
                mode,
            ),
            (
                L10nKey::SettingsContinueInterrupted,
                L10nKey::SettingsContinueInterruptedDesc,
                interrupted,
            ),
        ];
        if let Some(input) = cx
            .try_global::<ContinuePromptInputs>()
            .and_then(|i| i.0.get(&cx.entity_id()).cloned())
        {
            let field = self
                .settings_text_input(&input, 220., false, cx)
                .into_any_element();
            rows.push((
                L10nKey::SettingsContinuePrompt,
                L10nKey::SettingsContinuePromptDesc,
                field,
            ));
        }
        rows.into_iter()
            .map(|(title, desc, control)| {
                self.settings_row(t(title), t(desc), control, cx)
                    .into_any_element()
            })
            .collect()
    }

    /// The Continue message field, built with the settings page's other
    /// inputs against the window it is drawn in, and written to the config on
    /// Enter or when it loses focus.
    pub(crate) fn build_continue_prompt_input(
        &mut self,
        subs: &mut Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = cx.global::<Config>().fork.continue_prompt.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        subs.push(cx.subscribe_in(&input, window, |this, input, ev, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let value = input.read(cx).value().to_string();
                if cx.global::<Config>().fork.continue_prompt != value {
                    this.update_config(cx, |c| c.fork.continue_prompt = value);
                }
            }
        }));
        let id = cx.entity_id();
        cx.default_global::<ContinuePromptInputs>()
            .0
            .insert(id, input);
    }
}

/// What a leaf put to sleep records of `view`'s agent: its own, with the
/// session its mirrored tree record kept after an agent left filled in.
pub(crate) fn sleeping_agent(
    view: &crate::terminal::view::TerminalView,
    cx: &App,
) -> (Option<CLIAgent>, Option<String>, Option<Vec<String>>) {
    let session = view.agent_session();
    let live = view.agent().map(|agent| AgentFacts {
        agent,
        session_id: session.as_ref().and_then(|s| s.session_id.clone()),
        launch_argv: session.and_then(|s| s.launch_argv),
        status: None,
    });
    let last = crate::ui::machine_mirror::MachineMirrors::machine(cx, view.host_id())
        .and_then(|m| m.panes.iter().find(|p| p.id == view.pane_id))
        .and_then(|p| p.last_session.as_ref());
    match last_session::with_last_session(live, last) {
        Some(f) => (Some(f.agent), f.session_id, f.launch_argv),
        None => (None, None, None),
    }
}

/// Each window's Continue message field, by its app.
#[derive(Default)]
struct ContinuePromptInputs(HashMap<gpui::EntityId, Entity<InputState>>);

impl Global for ContinuePromptInputs {}

/// An agent with no session still comes back, started anew.
pub(crate) fn layout_has_agent(pane: &SessionPane) -> bool {
    match pane {
        SessionPane::Leaf { agent, .. } => agent.is_some(),
        SessionPane::Split { a, b, .. } => layout_has_agent(a) || layout_has_agent(b),
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
        LaunchWake, PaneResume, Reopen, RestoredDead, Resume, ResumePlan, Wake, agent_tabs,
        dead_agent, dead_agent_spawn, decide, flush_restored, forget_restored, launch_wake,
        layout_has_live_pane, load_restored, record_restores_asleep, remove_restored_file,
        restart_wakes, runs_as_program, take_launch_wake, take_restored, typed,
        with_minted_session,
    };
    use gpui::Entity;

    use crate::core::config::Config;
    use crate::core::session::{RemoteTarget, SessionAxis, SessionPane, SessionTab, WorkspaceId};
    use crate::daemon::protocol::ShellSpec;
    use crate::terminal::PaneWorkspace;
    use crate::terminal::view::{TerminalView, quiet_test_pane};
    use crate::ui::app::test_window::harness_with_tabs;
    use crate::ui::background_grid::Grid;
    use crate::ui::first_prompt::tests::report_prompt;
    use crate::ui::machine_mirror::MachineMirrors;
    use crate::ui::pending_pane::PendingSpawn;
    use crate::ui::wake_pool::Job;
    use tty7_core::core::cli_agent::CLIAgent;
    use tty7_core::core::fork_config::AgentResumeMode::{Program, Typed};
    use tty7_core::core::machine::{AgentFacts, Machine, PaneRecord, TabId};

    fn agent_leaf(session: Option<&str>) -> SessionPane {
        leaf(session.map(|_| CLIAgent::Claude), session)
    }

    fn leaf(agent: Option<CLIAgent>, session: Option<&str>) -> SessionPane {
        SessionPane::Leaf {
            cwd: None,
            pane_id: None,
            shell: None,
            ssh_spec: None,
            agent,
            agent_session_id: session.map(Into::into),
            agent_launch_argv: None,
        }
    }

    /// `view`'s record keeps `session` as the one its agent left.
    fn left_session(
        view: &Entity<TerminalView>,
        session: Option<&str>,
        vcx: &mut gpui::VisualTestContext,
    ) {
        vcx.update(|_, cx| {
            let view = view.read(cx);
            let mut record = PaneRecord::new(view.pane_id);
            record.last_session = Some(AgentFacts {
                agent: CLIAgent::Claude,
                session_id: session.map(Into::into),
                launch_argv: None,
                status: None,
            });
            let machine = Machine {
                panes: vec![record],
                ..Default::default()
            };
            MachineMirrors::install(cx, view.host_id(), machine);
        });
    }

    #[gpui::test]
    fn a_pane_put_to_sleep_keeps_the_session_its_agent_left(cx: &mut gpui::TestAppContext) {
        crate::core::config::pin_test_config_dir();
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(Config::default());
        });
        let mut vcx = cx.add_empty_window().clone();
        let (view, _daemon) = vcx.update(|window, cx| quiet_test_pane(41, window, cx));
        let read = |vcx: &mut gpui::VisualTestContext| {
            vcx.update(|_, cx| super::sleeping_agent(view.read(cx), cx))
        };
        assert_eq!(read(&mut vcx), (None, None, None));
        left_session(&view, Some("s-left"), &mut vcx);
        assert_eq!(
            read(&mut vcx),
            (Some(CLIAgent::Claude), Some("s-left".into()), None)
        );
    }

    #[gpui::test]
    fn a_record_with_no_session_resumes_the_one_its_argv_names(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_global(Config::default()));
        let id = "0b5c3a5e-6d0e-4c1f-9a4b-2f7f1d9e8c11";
        let argv: Vec<String> = ["node", "/bin/codex", "resume", id]
            .map(str::to_string)
            .into();
        let codex = Some(CLIAgent::Codex);
        cx.update(|cx| {
            let resumed = |session: Option<&str>, argv: &[String]| {
                Resume::restored(&codex, session, Some(argv), None, cx).map(|r| r.reopen)
            };
            let session = |id: &str| {
                Some(Reopen::Session {
                    session_id: id.into(),
                    prompt: None,
                    on_interrupt: None,
                })
            };
            assert_eq!(resumed(None, &argv), session(id));
            assert_eq!(
                resumed(Some("01a00000-0000-7000-8000-000000000000"), &argv),
                session("01a00000-0000-7000-8000-000000000000"),
                "a captured session wins over the argv's"
            );
            assert_eq!(
                resumed(None, &argv[..2]),
                Some(Reopen::Launch("codex".into())),
                "a fresh `codex` starts anew"
            );
        });
    }

    #[gpui::test]
    fn an_agent_that_died_to_an_idle_shell_resumes_its_last_session(cx: &mut gpui::TestAppContext) {
        crate::core::config::pin_test_config_dir();
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(Config::default());
        });
        let mut vcx = cx.add_empty_window().clone();
        let (view, mut daemon) = vcx.update(|window, cx| quiet_test_pane(42, window, cx));
        left_session(&view, Some("s-died"), &mut vcx);
        let read = |vcx: &mut gpui::VisualTestContext| {
            vcx.update(|_, cx| dead_agent(view.read(cx), Some("go"), cx).map(|r| r.resume))
        };
        assert_eq!(read(&mut vcx), None, "a shell not at its prompt is left be");

        report_prompt(&view, &mut vcx, &mut daemon);
        let resume = read(&mut vcx).expect("resumes at the prompt");
        assert_eq!(
            resume.reopen,
            Reopen::Session {
                session_id: "s-died".into(),
                prompt: Some("go".into()),
                on_interrupt: None,
            }
        );
        left_session(&view, None, &mut vcx);
        assert_eq!(
            read(&mut vcx),
            None,
            "an idle shell whose agent left no session is the user's"
        );
    }

    #[gpui::test]
    fn a_job_resumes_only_its_own_dead_panes_and_a_wake_goes_to_the_wake(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, mut vcx, mut streams) = harness_with_tabs(cx, 2);
        let views = app.update(&mut vcx, |app, _| {
            app.tabs
                .iter()
                .map(|t| t.pane.terminals()[0].clone())
                .collect::<Vec<_>>()
        });
        left_session(&views[0], Some("s-died"), &mut vcx);
        report_prompt(&views[0], &mut vcx, &mut streams[0]);
        let (dead, other) = (views[0].entity_id(), views[1].entity_id());
        app.update_in(&mut vcx, |app, window, cx| {
            let other = Job::ResumeDead(vec![other]);
            assert!(!app.start_job(0, &other, None, window, cx), "not its pane");
            let own = Job::ResumeDead(vec![dead]);
            assert!(app.start_job(0, &own, None, window, cx));

            let live = Job::ResumeDead(vec![views[1].entity_id()]);
            assert!(!app.start_job(1, &live, None, window, cx), "nothing dead");
            assert!(
                app.start_job(1, &Job::Wake, None, window, cx),
                "already awake"
            );
        });
    }

    #[test]
    fn a_wake_picks_only_sleeping_tabs_with_an_agent() {
        let agent = agent_leaf(Some("s1"));
        let shell = agent_leaf(None);
        let sessionless = leaf(Some(CLIAgent::Codex), None);
        let (asleep_agent, asleep_shell, awake) = (TabId::new(), TabId::new(), TabId::new());
        let asleep_new = TabId::new();
        let picked = agent_tabs([
            (asleep_agent, Some(&agent)),
            (asleep_shell, Some(&shell)),
            (asleep_new, Some(&sessionless)),
            (awake, None),
        ]);
        assert_eq!(picked, vec![asleep_agent, asleep_new]);
    }

    #[gpui::test]
    fn an_automatic_wake_leaves_hibernated_tabs_asleep(cx: &mut gpui::TestAppContext) {
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
            None,
            "off by default, as upstream"
        );
    }

    #[test]
    fn with_resume_off_restart_leaves_agent_tabs_asleep() {
        let mut cfg = Config::default();
        cfg.fork.resume_agents_on_launch = true;
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
        let session =
            |session_id: &str, prompt: Option<&str>, on_interrupt: Option<&str>| Reopen::Session {
                session_id: session_id.into(),
                prompt: prompt.map(Into::into),
                on_interrupt: on_interrupt.map(Into::into),
            };
        let resume = Resume {
            agent: CLIAgent::Claude,
            launch_argv: Some(vec!["claude".into(), "--model".into(), "opus".into()]),
            reopen: session(ID, None, Some("continue")),
        };
        assert_eq!(
            resume.line(ResumePlan::Resume, "zsh").as_deref(),
            Some(format!("claude --model opus --resume {ID}").as_str()),
            "an ended turn resumes quiet"
        );
        assert_eq!(
            resume.line(ResumePlan::Interrupted, "zsh").as_deref(),
            Some(format!("claude --model opus --resume {ID} continue").as_str()),
            "a cut-off turn is told to carry on"
        );
        let continue_all = Resume {
            reopen: session(ID, Some("go on"), Some("continue")),
            ..resume.clone()
        };
        assert_eq!(
            continue_all.line(ResumePlan::Resume, "zsh").as_deref(),
            Some(format!("claude --model opus --resume {ID} 'go on'").as_str()),
            "Continue All's prompt goes either way"
        );
        assert_eq!(
            resume.line(ResumePlan::Fresh, "zsh").as_deref(),
            Some(format!("claude --model opus --session-id {ID}").as_str())
        );
        assert_eq!(
            resume
                .line(ResumePlan::Attach("6011098d".into()), "zsh")
                .as_deref(),
            Some("claude attach 6011098d")
        );
        for plan in [ResumePlan::Resume, ResumePlan::Fresh] {
            assert!(!resume.line(plan, "zsh").unwrap().contains("||"));
        }

        let codex = Resume {
            agent: CLIAgent::Codex,
            launch_argv: None,
            reopen: session("th_1", None, None),
        };
        assert_eq!(
            codex.line(ResumePlan::Fresh, "zsh").as_deref(),
            Some("codex resume th_1"),
            "an agent that can't name a new session resumes"
        );
        let unsafe_id = Resume {
            reopen: session("$(boom)", None, None),
            ..resume
        };
        assert_eq!(unsafe_id.line(ResumePlan::Resume, "zsh"), None);
    }

    #[test]
    fn a_prompt_is_quoted_for_the_shell_it_lands_in() {
        let resume = Resume {
            agent: CLIAgent::Claude,
            launch_argv: None,
            reopen: Reopen::Session {
                session_id: "0b5c3a5e-6d0e-4c1f-9a4b-2f7f1d9e8c11".into(),
                prompt: Some("go `rm -rf ~` $HOME".into()),
                on_interrupt: None,
            },
        };
        let line = resume.line(ResumePlan::Resume, "/bin/zsh").unwrap();
        assert!(line.ends_with("'go `rm -rf ~` $HOME'"), "{line}");
    }

    /// #77: a woken agent opens at the window's grid, not the placeholder.
    #[gpui::test]
    fn a_dead_agent_spawns_at_the_window_s_grid(cx: &mut gpui::TestAppContext) {
        let leaf = SessionPane::Leaf {
            cwd: None,
            pane_id: Some(9),
            shell: None,
            ssh_spec: None,
            agent: Some(CLIAgent::Claude),
            agent_session_id: Some("0b5c3a5e-6d0e-4c1f-9a4b-2f7f1d9e8c11".into()),
            agent_launch_argv: None,
        };
        let grid = Grid::for_test(200, 50);
        let empty = Default::default();
        cx.update(|cx| {
            cx.set_global(Config::default());
            let spawn = dead_agent_spawn(
                &leaf,
                None,
                WorkspaceId::new(),
                &None,
                Some(9),
                Some(&empty),
                13.,
                Some(grid),
                cx,
            )
            .expect("a dead agent leaf respawns");
            assert_eq!(spawn.grid, Some(grid));
        });
    }

    #[gpui::test]
    fn an_agent_with_no_session_restores_started_anew(cx: &mut gpui::TestAppContext) {
        let leaf = |agent, argv: &[&str]| SessionPane::Leaf {
            cwd: None,
            pane_id: Some(9),
            shell: None,
            ssh_spec: None,
            agent: Some(agent),
            agent_session_id: None,
            agent_launch_argv: Some(argv.iter().map(|t| t.to_string()).collect()),
        };
        let empty = Default::default();
        cx.update(|cx| {
            let mut cfg = Config::default();
            cfg.agent_launch.insert(
                "claude".into(),
                "claude --dangerously-skip-permissions".into(),
            );
            cx.set_global(cfg);
            let restored = |leaf: &SessionPane| {
                let mut spawn = dead_agent_spawn(
                    leaf,
                    None,
                    WorkspaceId::new(),
                    &None,
                    Some(9),
                    Some(&empty),
                    13.,
                    None,
                    cx,
                )
                .expect("an agent with no session respawns");
                decide(&mut spawn, false, cx);
                spawn.resume.expect("its agent comes back").line()
            };
            assert_eq!(
                restored(&leaf(CLIAgent::Codex, &["node", "/bin/codex", "--yolo"])).as_deref(),
                Some("codex --yolo")
            );
            assert_eq!(
                restored(&leaf(CLIAgent::Pi, &["pi"])).as_deref(),
                Some("pi")
            );
            let claude = restored(&leaf(CLIAgent::Claude, &["claude"])).unwrap();
            assert!(
                claude.starts_with("claude --dangerously-skip-permissions --session-id "),
                "{claude}"
            );
        });
    }

    #[gpui::test]
    fn a_resume_goes_to_the_spawn_or_to_the_prompt_decided_once(cx: &mut gpui::TestAppContext) {
        let spawn = PendingSpawn {
            restore_pane: Some(7),
            shell: Some(ShellSpec {
                program: "/bin/zsh".into(),
                args: Vec::new(),
                args_are_tty7_defaults: false,
            }),
            agent: Some(CLIAgent::Claude),
            agent_session_id: Some("0b5c3a5e-6d0e-4c1f-9a4b-2f7f1d9e8c11".into()),
            ..Default::default()
        };
        cx.update(|cx| {
            let mut cfg = Config::default();
            cfg.fork.agent_resume_mode = Program;
            cx.set_global(cfg);
            let set =
                |cx: &mut gpui::App, mode| cx.global_mut::<Config>().fork.agent_resume_mode = mode;

            let mut program = spawn.clone();
            decide(&mut program, true, cx);
            let decided = program.resume.clone().expect("a resume");
            assert!(decided.run_once, "handed to the spawn");
            assert!(decided.line().is_some_and(|l| l.starts_with("claude ")));
            set(cx, Typed);
            decide(&mut program, true, cx);
            assert!(
                program.resume.as_ref().unwrap().run_once,
                "a retry keeps it"
            );
            assert!(typed(&program).is_none(), "so the landing types nothing");

            let mut typing = spawn.clone();
            decide(&mut typing, true, cx);
            set(cx, Program);
            assert!(typed(&typing).is_some(), "decided typed, typed it stays");

            let mut old_daemon = spawn.clone();
            decide(&mut old_daemon, false, cx);
            assert!(
                typed(&old_daemon).is_some(),
                "a daemon without run-once is typed into"
            );

            let mut wsl = spawn.clone();
            wsl.shell.as_mut().unwrap().program = "wsl.exe".into();
            decide(&mut wsl, true, cx);
            assert!(
                typed(&wsl).is_some_and(|r| !r.local),
                "WSL's files are not here"
            );

            assert!(PaneResume::of(&PendingSpawn::default(), true, cx).is_none());
        });
    }

    #[test]
    fn a_resume_runs_as_the_tab_only_where_it_can() {
        assert!(runs_as_program(Program, true, Some(7), "/bin/zsh"));
        for shell in ["sh", "/opt/homebrew/bin/bash", "dash", "ksh", "fish"] {
            assert!(runs_as_program(Program, true, Some(7), shell), "{shell}");
        }
        assert!(
            !runs_as_program(Typed, true, Some(7), "/bin/zsh"),
            "the default types"
        );
        assert!(
            !runs_as_program(Program, false, Some(7), "/bin/zsh"),
            "a remote pane types"
        );
        assert!(
            !runs_as_program(Program, true, None, "/bin/zsh"),
            "no pane to replace"
        );
        for shell in ["nu", "pwsh", "wsl.exe", "cmd", "xonsh"] {
            assert!(
                !runs_as_program(Program, true, Some(7), shell),
                "{shell} is typed into"
            );
        }
    }
}
