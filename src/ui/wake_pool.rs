//! Waking agent tabs a few at a time. Continue All Agents and the launch and
//! restart wakes queue their tabs here, and at most `agent_wake_concurrency`
//! of them start at once: a tab holds its slot until its agent reports a
//! session or has been in the foreground for [`FOREGROUND_UP`], and never
//! past [`START_TIMEOUT`]. Queued and starting tabs wear a spinner in place of
//! their sleep mark.

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt as _, AnyElement, AnyWindowHandle, App, AsyncApp, Context, EntityId,
    Global, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div, px,
};
use gpui_component::{Icon, IconName};

use crate::core::config::Config;
use crate::ui::app::{Tab, Tty7App};
use crate::ui::settings::kit::{self, Tk};
use crate::ui::windows::WindowRegistry;
use tty7_core::core::machine::TabId;

/// The hard cap on how long a starting tab holds its slot.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// An agent that never reports a session (no hooks) counts as up once it has
/// held the foreground this long.
const FOREGROUND_UP: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(500);
const MAX: u8 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Queued,
    Starting,
}

/// What a tick saw of a tab in the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    /// Closed, or its window is.
    Gone,
    /// Still asleep.
    Asleep,
    /// Awake with no agent in the foreground yet.
    Awake,
    Foreground,
    /// Its agent reported a session.
    Session,
}

/// What a tab in the pool is started with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Job {
    /// Wake it from sleep.
    Wake,
    /// Resume the agents that died to their shell in these panes.
    ResumeDead(Vec<EntityId>),
}

impl Job {
    fn covers(&self, pane: EntityId) -> bool {
        match self {
            Job::Wake => true,
            Job::ResumeDead(panes) => panes.contains(&pane),
        }
    }
}

struct Slot {
    since: Instant,
    foreground: Option<Instant>,
    /// The session each pane's agent had reported when the pool first saw it.
    reported: HashMap<EntityId, Option<String>>,
}

/// A pane as a tick sees it: whether an agent is in front, whether that
/// agent has a session, and the session its hooks reported, if any.
type Leaf = (EntityId, bool, bool, Option<String>);

/// The queue, the slots, and what each tab is started with (`T`, the window
/// it lives in, its job, and its prompt).
struct Pool<T> {
    queue: VecDeque<TabId>,
    starting: HashMap<TabId, Slot>,
    jobs: HashMap<TabId, (T, Job, Option<String>)>,
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            starting: HashMap::new(),
            jobs: HashMap::new(),
        }
    }
}

impl<T: Clone> Pool<T> {
    /// A tab already queued keeps its place, and takes `prompt` when it brings
    /// one; a starting tab is left alone.
    fn enqueue(
        &mut self,
        jobs: impl IntoIterator<Item = (TabId, Job)>,
        at: T,
        prompt: Option<String>,
    ) {
        for (id, job) in jobs {
            match self.mark(id) {
                Some(Mark::Starting) => {}
                Some(Mark::Queued) if prompt.is_none() => {}
                Some(Mark::Queued) => {
                    self.jobs.insert(id, (at.clone(), job, prompt.clone()));
                }
                None => {
                    self.queue.push_back(id);
                    self.jobs.insert(id, (at.clone(), job, prompt.clone()));
                }
            }
        }
    }

    /// Moves queued tabs into the free slots, and returns them to be started.
    fn admit(&mut self, cap: usize, now: Instant) -> Vec<(TabId, T, Job, Option<String>)> {
        let mut admitted = Vec::new();
        while self.starting.len() < cap
            && let Some(id) = self.queue.pop_front()
        {
            let Some((at, job, prompt)) = self.jobs.get(&id).cloned() else {
                continue;
            };
            self.starting.insert(
                id,
                Slot {
                    since: now,
                    foreground: None,
                    reported: HashMap::new(),
                },
            );
            admitted.push((id, at, job, prompt));
        }
        admitted
    }

    /// Takes in what a tick saw of tab `id`; true when that ends its stay.
    /// A queued tab leaves once it is gone or something else did its job: woke
    /// it, or brought its dead agent up; a starting one once its agent is up
    /// or its time is out.
    fn observe(&mut self, id: TabId, seen: Seen, now: Instant) -> bool {
        let done = match (self.mark(id), seen) {
            (None, _) => return false,
            // Asleep while starting: its wake failed or it was put back to
            // sleep, and either way nothing is coming up.
            (_, Seen::Gone) | (Some(Mark::Starting), Seen::Session | Seen::Asleep) => true,
            (Some(Mark::Queued), seen) => match self.jobs.get(&id) {
                // Awake from the start, so only its agent coming up counts.
                Some((_, Job::ResumeDead(_), _)) => {
                    matches!(seen, Seen::Foreground | Seen::Session)
                }
                _ => seen != Seen::Asleep,
            },
            (Some(Mark::Starting), seen) => {
                let slot = self.starting.get_mut(&id).expect("starting");
                match seen {
                    Seen::Foreground => {
                        slot.foreground.get_or_insert(now);
                    }
                    _ => slot.foreground = None,
                }
                now.duration_since(slot.since) >= START_TIMEOUT
                    || slot
                        .foreground
                        .is_some_and(|f| now.duration_since(f) >= FOREGROUND_UP)
            }
        };
        if done {
            self.finish(id);
        }
        done
    }

    /// What tab `id` adds up to, from whether it sleeps and its panes (`None`
    /// when it is gone). A starting tab is up only on a session its agent
    /// reported after the pool first saw the pane: an id read off the resume's
    /// own argv, or the status the daemon replays on attach, was there before
    /// this start.
    fn sight(&mut self, id: TabId, sight: Option<(bool, Vec<Leaf>)>) -> Seen {
        let Some((asleep, leaves)) = sight else {
            return Seen::Gone;
        };
        if asleep {
            return Seen::Asleep;
        }
        let mut slot = self.starting.get_mut(&id);
        let leaves: Vec<(bool, bool)> = leaves
            .into_iter()
            .map(
                |(pane, agent, session, reported)| match slot.as_deref_mut() {
                    Some(slot) => {
                        let before = slot.reported.entry(pane).or_insert(reported.clone());
                        (agent, reported.is_some() && *before != reported)
                    }
                    None => (agent, session),
                },
            )
            .collect();
        classify(false, leaves)
    }

    fn finish(&mut self, id: TabId) {
        self.starting.remove(&id);
        self.queue.retain(|q| *q != id);
        self.jobs.remove(&id);
    }

    fn mark(&self, id: TabId) -> Option<Mark> {
        if self.starting.contains_key(&id) {
            Some(Mark::Starting)
        } else if self.queue.contains(&id) {
            Some(Mark::Queued)
        } else {
            None
        }
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty() && self.starting.is_empty()
    }
}

type Window = (WeakEntity<Tty7App>, AnyWindowHandle);

type Start = fn(TabId, &Window, &Job, Option<&str>, &mut App) -> bool;

struct WakePool {
    pool: Pool<Window>,
    polling: bool,
    /// [`start`]; a test swaps in a recorder.
    start: Start,
}

impl Default for WakePool {
    fn default() -> Self {
        Self {
            pool: Pool::default(),
            polling: false,
            start,
        }
    }
}

impl Global for WakePool {}

/// The `agent_wake_concurrency` in force: the setting, else a quarter of the
/// cores, 1 to 20 either way.
pub(crate) fn concurrency(cfg: &Config) -> usize {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    concurrency_for(cfg.fork.agent_wake_concurrency, cores)
}

fn concurrency_for(set: Option<u8>, cores: usize) -> usize {
    set.map_or(cores / 4, usize::from).clamp(1, MAX.into())
}

/// Queue `jobs` of the window `app` draws in to be started with `prompt`.
/// Safe inside that window's own update: nothing starts until it is over.
pub(crate) fn queue(
    jobs: Vec<(TabId, Job)>,
    prompt: Option<String>,
    window: AnyWindowHandle,
    cx: &mut Context<Tty7App>,
) {
    let at = (cx.entity().downgrade(), window);
    log::info!("wake pool: queueing {} tab(s)", jobs.len());
    cx.default_global::<WakePool>()
        .pool
        .enqueue(jobs, at, prompt);
    cx.defer(pump);
}

/// Whether tab `id` is queued or starting.
pub(crate) fn is_waking(id: TabId, cx: &App) -> bool {
    cx.try_global::<WakePool>()
        .is_some_and(|w| w.pool.mark(id).is_some())
}

/// The mark a tab wears for its sleep state: the spinner while it is queued
/// or starting, else the moon while it sleeps.
pub(crate) fn state_mark(
    app: &Tty7App,
    id: impl Into<gpui::ElementId>,
    tab: &Tab,
    cx: &App,
) -> Option<AnyElement> {
    if is_waking(tab.tree_id.get(), cx) {
        Some(spinner(id, cx))
    } else {
        tab.is_asleep().then(|| app.sleep_mark(id, cx))
    }
}

fn pump(cx: &mut App) {
    let cap = concurrency(cx.global::<Config>());
    let admitted = cx
        .default_global::<WakePool>()
        .pool
        .admit(cap, Instant::now());
    let changed = !admitted.is_empty();
    let start = cx.global::<WakePool>().start;
    for (id, at, job, prompt) in admitted {
        let pool = &cx.global::<WakePool>().pool;
        log::info!(
            "wake pool: starting {id} ({}/{cap} starting, {} queued)",
            pool.starting.len(),
            pool.queue.len()
        );
        if !start(id, &at, &job, prompt.as_deref(), cx) {
            cx.default_global::<WakePool>().pool.finish(id);
        }
    }
    if changed {
        redraw(cx);
    }
    let state = cx.default_global::<WakePool>();
    if state.polling || state.pool.is_empty() {
        return;
    }
    state.polling = true;
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            smol::Timer::after(POLL).await;
            if !cx.update(tick) {
                return;
            }
        }
    })
    .detach();
}

/// Starts tab `id`'s job; false when it is gone or could not be started.
fn start(id: TabId, (app, window): &Window, job: &Job, prompt: Option<&str>, cx: &mut App) -> bool {
    let woke = window.update(cx, |_, window, cx| {
        app.update(cx, |this, cx| {
            let i = this.tabs.iter().position(|t| t.tree_id.get() == id)?;
            Some(this.start_job(i, job, prompt, window, cx))
        })
    });
    matches!(woke, Ok(Ok(Some(true))))
}

/// Lets go of tabs that came up, went away or ran out of time, and fills
/// their slots. Answers whether to keep polling.
fn tick(cx: &mut App) -> bool {
    let now = Instant::now();
    let pool = &cx.global::<WakePool>().pool;
    let sights: Vec<_> = pool
        .jobs
        .iter()
        .map(|(id, ((app, _), job, _))| (*id, see(*id, app, job, cx)))
        .collect();
    let pool = &mut cx.global_mut::<WakePool>().pool;
    let mut changed = false;
    for (id, sight) in sights {
        let seen = pool.sight(id, sight);
        if pool.observe(id, seen, now) {
            changed = true;
            log::info!(
                "wake pool: {id} done ({seen:?}), {} starting, {} queued",
                pool.starting.len(),
                pool.queue.len()
            );
        }
    }
    if changed {
        redraw(cx);
    }
    pump(cx);
    let state = cx.default_global::<WakePool>();
    state.polling = !state.pool.is_empty();
    state.polling
}

/// Tab `id` as `job` sees it: a resume looks only at its own panes, so a live
/// agent in a split beside them does not count as theirs.
fn see(id: TabId, app: &WeakEntity<Tty7App>, job: &Job, cx: &App) -> Option<(bool, Vec<Leaf>)> {
    let app = app.upgrade()?;
    let tab = app.read(cx).tabs.iter().find(|t| t.tree_id.get() == id)?;
    let leaves = tab
        .pane
        .terminals()
        .iter()
        .filter(|v| job.covers(v.entity_id()))
        .map(|view| {
            let session = view.read(cx).agent_session();
            let reported = session
                .as_ref()
                .filter(|s| s.rich)
                .and_then(|s| s.session_id.clone());
            let agent = view.read(cx).agent().is_some();
            (view.entity_id(), agent, session.is_some(), reported)
        })
        .collect();
    Some((tab.is_asleep(), leaves))
}

/// What a tab is, from whether it sleeps and, for each of its panes, whether
/// an agent is in the foreground and whether that agent reported a session.
fn classify(asleep: bool, leaves: impl IntoIterator<Item = (bool, bool)>) -> Seen {
    if asleep {
        return Seen::Asleep;
    }
    let mut seen = Seen::Awake;
    for (agent, session) in leaves {
        match (agent, session) {
            (true, true) => return Seen::Session,
            (true, false) => seen = Seen::Foreground,
            _ => {}
        }
    }
    seen
}

fn redraw(cx: &mut App) {
    if !cx.has_global::<WindowRegistry>() {
        return;
    }
    for (_, app) in WindowRegistry::open_windows(cx) {
        let _ = app.update(cx, |_, cx| cx.notify());
    }
}

fn spinner(id: impl Into<gpui::ElementId>, cx: &App) -> AnyElement {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .size(px(16.))
        .text_color(gpui_component::ActiveTheme::theme(cx).muted_foreground)
        .child(
            Icon::new(IconName::LoaderCircle)
                .size(px(11.))
                .with_animation(
                    id,
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |icon, delta| {
                        icon.transform(gpui::Transformation::rotate(gpui::percentage(delta)))
                    },
                ),
        )
        .into_any_element()
}

/// The Agents starting at once slider.
pub(crate) fn concurrency_slider(cx: &mut Context<Tty7App>) -> AnyElement {
    let n = concurrency(cx.global::<Config>());
    let app = cx.entity().downgrade();
    let release_app = app.clone();
    kit::slider(
        "agent-wake-concurrency",
        (n - 1) as f32 / f32::from(MAX - 1),
        n.to_string(),
        &Tk::of(cx),
        Rc::new(move |f, _window, cx| {
            let n = (1. + f * f32::from(MAX - 1)).round() as u8;
            cx.global_mut::<Config>().fork.agent_wake_concurrency = Some(n.clamp(1, MAX));
            let _ = app.update(cx, |_, cx| cx.notify());
        }),
        Rc::new(move |_window, cx| {
            let _ = release_app.update(cx, |this, cx| this.persist_settings_config(cx));
            // A raised cap fills its new slots, after the event, not in it.
            cx.defer(pump);
        }),
    )
}

#[cfg(test)]
pub(super) mod tests {
    use std::time::{Duration, Instant};

    use gpui::Global;

    use super::{
        FOREGROUND_UP, Job, Mark, Pool, START_TIMEOUT, Seen, WakePool, Window, classify,
        concurrency_for, is_waking, queue, tick,
    };
    use crate::core::config::Config;
    use crate::core::session::{SessionPane, SessionTab};
    use crate::ui::agent_resume::{Wake, record_restores_asleep};
    use crate::ui::app::test_window::harness_with_tabs;
    use tty7_core::core::cli_agent::CLIAgent;
    use tty7_core::core::machine::TabId;

    /// What the pool was asked to wake, in place of waking it.
    #[derive(Default)]
    struct Started(Vec<(TabId, Option<String>)>);

    impl Global for Started {}

    fn record(id: TabId, _: &Window, _: &Job, prompt: Option<&str>, cx: &mut gpui::App) -> bool {
        let started = &mut cx.default_global::<Started>().0;
        started.push((id, prompt.map(str::to_string)));
        true
    }

    fn recording(cx: &mut gpui::App) {
        cx.default_global::<WakePool>().start = record;
        cx.set_global(Started::default());
    }

    fn ids(admitted: Vec<(TabId, (), Job, Option<String>)>) -> Vec<TabId> {
        admitted.into_iter().map(|(id, ..)| id).collect()
    }

    fn wake(ids: impl IntoIterator<Item = TabId>) -> Vec<(TabId, Job)> {
        ids.into_iter().map(|id| (id, Job::Wake)).collect()
    }

    #[test]
    fn a_tab_is_up_once_an_agent_reports_a_session_in_any_pane() {
        assert_eq!(classify(true, [(true, true)]), Seen::Asleep);
        assert_eq!(classify(false, []), Seen::Awake);
        assert_eq!(classify(false, [(false, false)]), Seen::Awake);
        assert_eq!(
            classify(false, [(false, false), (true, false)]),
            Seen::Foreground
        );
        assert_eq!(
            classify(false, [(true, false), (true, true)]),
            Seen::Session
        );
    }

    /// A tab used to leave its slot a tick after it started, on the session
    /// its resume's argv named or the daemon replayed, long before Codex did.
    #[test]
    fn a_starting_tab_is_up_only_on_a_session_reported_after_it_started() {
        let (tab, queued) = (TabId::new(), TabId::new());
        let pane = gpui::EntityId::from(1u64);
        let mut pool = Pool::default();
        pool.enqueue(
            [tab, queued].map(|id| (id, Job::ResumeDead(vec![pane]))),
            (),
            None,
        );
        pool.admit(1, Instant::now());
        let leaf = |reported: Option<&str>| {
            Some((
                false,
                vec![(pane, true, true, reported.map(str::to_string))],
            ))
        };
        assert_eq!(
            pool.sight(queued, leaf(None)),
            Seen::Session,
            "a queued tab's agent is up by any session"
        );
        assert_eq!(
            pool.sight(tab, leaf(Some("s1"))),
            Seen::Foreground,
            "replayed"
        );
        assert_eq!(pool.sight(tab, leaf(Some("s1"))), Seen::Foreground);
        assert_eq!(pool.sight(tab, leaf(Some("s2"))), Seen::Session, "reported");

        let fresh = TabId::new();
        pool.enqueue([(fresh, Job::Wake)], (), None);
        pool.finish(tab);
        pool.admit(2, Instant::now());
        assert_eq!(pool.sight(fresh, leaf(None)), Seen::Foreground, "argv only");
        assert_eq!(pool.sight(fresh, leaf(Some("s1"))), Seen::Session);

        assert_eq!(pool.sight(fresh, None), Seen::Gone);
        assert_eq!(pool.sight(fresh, Some((true, Vec::new()))), Seen::Asleep);
    }

    #[test]
    fn a_starting_tab_put_back_to_sleep_frees_its_slot() {
        let tab = TabId::new();
        let mut pool = Pool::default();
        pool.enqueue(wake([tab]), (), None);
        pool.admit(1, Instant::now());
        assert!(pool.observe(tab, Seen::Asleep, Instant::now()));
        assert!(pool.is_empty());
    }

    #[gpui::test]
    fn restored_dead_tabs_spin_from_the_frame_they_land(cx: &mut gpui::TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 1);
        vcx.update(|_, cx| {
            cx.global_mut::<Config>().fork.agent_wake_concurrency = Some(1);
            recording(cx);
        });
        let dead = |n: u64| SessionTab {
            name: None,
            pane: SessionPane::Leaf {
                cwd: None,
                pane_id: Some(900 + n),
                shell: None,
                ssh_spec: None,
                agent: Some(CLIAgent::Claude),
                agent_session_id: Some(format!("s{n}")),
                agent_launch_argv: None,
            },
            group: None,
            last_auto: None,
            tree_id: Some(TabId::new()),
            hibernated: false,
            asleep_view: None,
        };
        let tabs = app.update_in(&mut vcx, |app, window, cx| {
            let empty = Default::default();
            let mut ids = Vec::new();
            for n in 0..2 {
                let st = dead(n);
                assert!(record_restores_asleep(None, Some(&empty), &st, cx));
                ids.push(st.tree_id.unwrap());
                app.tabs.push(crate::ui::app::asleep_tab(&st, None));
            }
            app.wake_restored(Wake, window, cx);
            assert!(
                ids.iter().all(|id| is_waking(*id, cx)),
                "spinning before anything is drawn"
            );
            ids
        });
        vcx.run_until_parked();
        let started = vcx.update(|_, cx| cx.global::<Started>().0.clone());
        assert_eq!(started, vec![(tabs[0], None)], "one at a time");
        crate::ui::agent_resume::remove_restored_file();
    }

    #[test]
    fn the_pool_never_starts_more_than_its_cap() {
        let tabs: Vec<TabId> = (0..5).map(|_| TabId::new()).collect();
        let mut pool = Pool::default();
        let now = Instant::now();
        pool.enqueue(wake(tabs.clone()), (), None);
        pool.enqueue(wake([tabs[0]]), (), None);
        let first = pool.admit(2, now);
        assert_eq!(ids(first), tabs[..2]);
        assert!(pool.admit(2, now).is_empty(), "both slots are held");
        assert_eq!(pool.mark(tabs[0]), Some(Mark::Starting));
        assert_eq!(pool.mark(tabs[2]), Some(Mark::Queued));

        assert!(pool.observe(tabs[0], Seen::Session, now));
        assert_eq!(pool.mark(tabs[0]), None, "up: back to its own icon");
        let next = pool.admit(2, now);
        assert_eq!(ids(next), tabs[2..3], "one slot freed, one start");
        assert_eq!(pool.starting.len(), 2);
    }

    #[test]
    fn a_slot_frees_on_a_foreground_agent_and_on_the_hard_cap() {
        let (quiet, stuck, next) = (TabId::new(), TabId::new(), TabId::new());
        let mut pool = Pool::default();
        let then = Instant::now();
        pool.enqueue(wake([quiet, stuck, next]), (), None);
        pool.admit(2, then);

        assert!(!pool.observe(quiet, Seen::Foreground, then));
        assert!(!pool.observe(quiet, Seen::Awake, then + FOREGROUND_UP));
        let back = then + FOREGROUND_UP;
        assert!(
            !pool.observe(quiet, Seen::Foreground, back),
            "the clock restarts"
        );
        assert!(pool.observe(quiet, Seen::Foreground, back + FOREGROUND_UP));

        assert!(!pool.observe(stuck, Seen::Awake, then + Duration::from_secs(1)));
        assert!(pool.observe(stuck, Seen::Awake, then + START_TIMEOUT));
        assert_eq!(pool.admit(2, then).len(), 1);
        assert!(pool.observe(next, Seen::Gone, then));
        assert!(pool.is_empty());
    }

    #[test]
    fn a_queued_tab_leaves_once_something_else_did_its_job_and_a_requeue_keeps_its_prompt() {
        let (busy, woken, dead, prompted) =
            (TabId::new(), TabId::new(), TabId::new(), TabId::new());
        let mut pool = Pool::default();
        let now = Instant::now();
        pool.enqueue(wake([busy, woken, prompted]), (), None);
        pool.enqueue([(dead, Job::ResumeDead(Vec::new()))], (), None);
        pool.admit(1, now);
        assert!(!pool.observe(woken, Seen::Asleep, now));
        assert!(pool.observe(woken, Seen::Awake, now), "woken by hand");
        assert!(
            !pool.observe(dead, Seen::Awake, now),
            "awake, nothing up yet"
        );
        assert!(pool.observe(dead, Seen::Foreground, now), "resumed by hand");

        pool.enqueue(wake([prompted, busy]), (), Some("go".into()));
        pool.enqueue(wake([prompted]), (), None);
        assert_eq!(pool.jobs[&prompted].2.as_deref(), Some("go"));
        assert_eq!(pool.jobs[&busy].2, None, "a starting tab is left alone");
    }

    #[test]
    fn a_queued_asleep_tab_woken_by_hand_is_not_resumed_on_its_turn() {
        let (busy, woken) = (TabId::new(), TabId::new());
        let mut pool = Pool::default();
        let now = Instant::now();
        pool.enqueue(wake([busy, woken]), (), None);
        pool.admit(1, now);
        assert!(pool.observe(woken, Seen::Awake, now), "woken by hand");
        assert!(pool.observe(busy, Seen::Session, now));
        assert!(
            pool.admit(1, now).is_empty(),
            "its resume is not typed twice"
        );
    }

    #[test]
    fn a_quarter_of_the_cores_by_default_and_1_to_20_always() {
        assert_eq!(concurrency_for(None, 16), 4);
        assert_eq!(concurrency_for(None, 2), 1);
        assert_eq!(concurrency_for(None, 256), 20);
        assert_eq!(concurrency_for(Some(7), 16), 7);
        assert_eq!(concurrency_for(Some(0), 16), 1);
        assert_eq!(concurrency_for(Some(50), 16), 20);
    }

    #[gpui::test]
    fn one_at_a_time_closing_the_starting_tab_admits_the_next(cx: &mut gpui::TestAppContext) {
        let (app, mut vcx, _streams) = harness_with_tabs(cx, 4);
        vcx.update(|_, cx| {
            cx.global_mut::<Config>().fork.agent_wake_concurrency = Some(1);
            recording(cx);
        });
        let started = |vcx: &mut gpui::VisualTestContext| {
            vcx.update(|_, cx| cx.global::<Started>().0.clone())
        };
        let tabs = app.update_in(&mut vcx, |app, window, cx| {
            for i in 1..4 {
                app.put_to_sleep(i, window, cx);
            }
            let tabs: Vec<TabId> = app.tabs[1..].iter().map(|t| t.tree_id.get()).collect();
            queue(wake(tabs.clone()), None, window.window_handle(), cx);
            tabs
        });
        vcx.run_until_parked();
        assert_eq!(started(&mut vcx), vec![(tabs[0], None)], "cap 1: one wake");
        vcx.update(|_, cx| {
            assert!(
                tabs.iter().all(|t| is_waking(*t, cx)),
                "every queued tab spins, the waiting ones too"
            )
        });

        app.update_in(&mut vcx, |_, window, cx| {
            queue(
                wake([tabs[2]]),
                Some("go".into()),
                window.window_handle(),
                cx,
            )
        });
        vcx.run_until_parked();
        for (closing, now_started) in [(1, tabs[1]), (1, tabs[2])] {
            app.update_in(&mut vcx, |app, window, cx| {
                app.close_tab(closing, window, cx)
            });
            vcx.update(|_, cx| tick(cx));
            assert_eq!(started(&mut vcx).last().map(|s| s.0), Some(now_started));
        }
        assert_eq!(
            started(&mut vcx),
            vec![
                (tabs[0], None),
                (tabs[1], None),
                (tabs[2], Some("go".into()))
            ],
            "the requeue's prompt reaches the wake"
        );
    }
}
