//! Fork (spawn-trace): see `docs/fork/features/spawn-trace.md`.

use std::cell::Cell;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    // GUI
    TabRequested,
    NewTabPageOpened,
    Picked,
    SpawnSent,
    Spawned,
    FirstPaint,
    // daemon
    ChildStarted,
    Prompt,
    Command,
    // both
    FirstOutput,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::TabRequested => "tab_requested",
            Stage::NewTabPageOpened => "new_tab_page_opened",
            Stage::Picked => "picked",
            Stage::SpawnSent => "spawn_sent",
            Stage::Spawned => "spawned",
            Stage::FirstPaint => "first_paint",
            Stage::ChildStarted => "child_started",
            Stage::Prompt => "prompt",
            Stage::Command => "command",
            Stage::FirstOutput => "first_output",
        }
    }
}

// `Trace::seen` is a u16 bitmask.
const _: () = assert!((Stage::FirstOutput as u16) < 16);

/// One tab's clock in one process.
#[derive(Debug)]
pub struct Trace {
    attempt: Option<u64>,
    pane: Option<u64>,
    t0: Instant,
    seen: u16,
}

impl Trace {
    /// The GUI's: numbered before any pane exists.
    pub fn attempt() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self::new(Some(NEXT.fetch_add(1, Ordering::Relaxed)), None)
    }

    /// The daemon's, from the pane's id.
    pub fn pane(id: u64) -> Self {
        Self::new(None, Some(id))
    }

    fn new(attempt: Option<u64>, pane: Option<u64>) -> Self {
        Self {
            attempt,
            pane,
            t0: Instant::now(),
            seen: 0,
        }
    }

    pub fn seen(&self, stage: Stage) -> bool {
        self.seen & (1 << stage as u16) != 0
    }

    /// Logs `stage` the first time only; the line it logged, if it did.
    pub fn stage(&mut self, stage: Stage, extra: &str) -> Option<String> {
        if self.seen(stage) {
            return None;
        }
        self.seen |= 1 << stage as u16;
        let mut line = String::from("spawn-trace");
        if let Some(a) = self.attempt {
            let _ = write!(line, " attempt={a}");
        }
        if let Some(p) = self.pane {
            let _ = write!(line, " pane={p}");
        }
        let ms = self.t0.elapsed().as_secs_f64() * 1000.0;
        let _ = write!(line, " stage={} ms={ms:.1}", stage.name());
        if !extra.is_empty() {
            line.push(' ');
            line.push_str(extra);
        }
        log::info!("{line}");
        Some(line)
    }

    /// GUI: the daemon answered with `pane`.
    pub fn spawned(mut self, pane: u64) -> Self {
        self.pane = Some(pane);
        self.stage(Stage::Spawned, "");
        self
    }

    /// Daemon reader: one read's worth. False once the last stage is logged,
    /// so the reader drops the trace and pays nothing more.
    pub fn read(&mut self, prompt: bool, command: bool) -> bool {
        self.stage(Stage::FirstOutput, "");
        if prompt {
            self.stage(Stage::Prompt, "");
        }
        if command {
            self.stage(Stage::Command, "");
        }
        !self.seen(Stage::Command)
    }
}

/// GUI paint: logs the first frame drawn after output arrived, then ends the
/// trace.
pub fn painted(trace: &mut Option<Trace>) {
    if let Some(t) = trace
        && t.seen(Stage::FirstOutput)
    {
        t.stage(Stage::FirstPaint, "");
        *trace = None;
    }
}

// Daemon: `DaemonPane::spawn` parks a pane's trace for its reader thread to
// pick up, so the shared reader signature doesn't have to carry it.
static PARKED: Mutex<Vec<Trace>> = Mutex::new(Vec::new());

pub fn park(trace: Trace) {
    PARKED.lock().unwrap().push(trace);
}

pub fn unpark(pane: u64) -> Option<Trace> {
    let mut parked = PARKED.lock().unwrap();
    let at = parked.iter().position(|t| t.pane == Some(pane))?;
    Some(parked.swap_remove(at))
}

// GUI: the attempt in flight on the UI thread, handed from the New Tab page
// (or `new_tab_slot`) down to `new_terminal`.
thread_local! {
    static HANDED: Cell<Option<Trace>> = const { Cell::new(None) };
}

/// Hands `trace` to the `new_terminal` that `f` reaches; a spawn refused
/// before it leaves the trace behind, so it is cleared after.
pub fn handing<R>(trace: Option<Trace>, f: impl FnOnce() -> R) -> R {
    HANDED.set(trace);
    let r = f();
    HANDED.set(None);
    r
}

pub fn take_handed() -> Option<Trace> {
    HANDED.take()
}

/// `new_tab_slot` without the page in front: a fresh attempt, unless the
/// page already handed one off.
pub fn request_tab() {
    let handed = HANDED.take().or_else(|| {
        let mut t = Trace::attempt();
        t.stage(Stage::TabRequested, "");
        Some(t)
    });
    HANDED.set(handed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_stages_log_once_in_order_then_the_trace_ends() {
        let mut t = Trace::pane(7);
        let first = t.stage(Stage::ChildStarted, "").unwrap();
        assert!(first.starts_with("spawn-trace pane=7 stage=child_started ms="));
        assert!(t.stage(Stage::ChildStarted, "").is_none());
        assert!(t.read(false, false));
        assert!(t.seen(Stage::FirstOutput) && !t.seen(Stage::Prompt));
        assert!(t.read(true, false));
        assert!(t.seen(Stage::Prompt) && !t.seen(Stage::Command));
        assert!(!t.read(true, true), "the command is the last stage");
        assert!(t.stage(Stage::FirstOutput, "").is_none());
        assert!(t.stage(Stage::Prompt, "").is_none());
    }

    #[test]
    fn gui_lines_carry_the_attempt_then_the_pane() {
        let mut t = Trace::attempt();
        let picked = t.stage(Stage::Picked, "kind=terminal dir=row").unwrap();
        let attempt = picked
            .split(' ')
            .find_map(|w| w.strip_prefix("attempt="))
            .unwrap()
            .to_string();
        assert!(picked.ends_with(" kind=terminal dir=row"), "{picked}");
        let mut t = t.spawned(42);
        let out = t.stage(Stage::FirstOutput, "").unwrap();
        assert!(
            out.starts_with(&format!(
                "spawn-trace attempt={attempt} pane=42 stage=first_output ms="
            )),
            "{out}"
        );
    }

    #[test]
    fn paint_waits_for_output_then_ends_the_trace() {
        let mut trace = Some(Trace::attempt().spawned(1));
        painted(&mut trace);
        assert!(trace.is_some(), "an empty first frame is not the one");
        trace.as_mut().unwrap().stage(Stage::FirstOutput, "");
        painted(&mut trace);
        assert!(trace.is_none());
    }

    #[test]
    fn a_parked_trace_is_found_by_pane_and_taken_once() {
        let mut t = Trace::pane(9001);
        t.stage(Stage::ChildStarted, "");
        park(t);
        assert!(unpark(9001).unwrap().seen(Stage::ChildStarted));
        assert!(unpark(9001).is_none());
    }

    #[test]
    fn request_tab_keeps_a_handed_off_attempt() {
        let page = Trace::attempt();
        let attempt = page.attempt;
        let kept = handing(Some(page), || {
            request_tab();
            take_handed().unwrap().attempt
        });
        assert_eq!(kept, attempt);
        handing(None, || {
            request_tab();
            assert!(take_handed().unwrap().seen(Stage::TabRequested));
        });
        assert!(take_handed().is_none());
    }
}
