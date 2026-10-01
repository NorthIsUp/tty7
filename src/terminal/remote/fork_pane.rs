//! The fork's state on a pane's link, kept out of `remote.rs` so rebasing on
//! upstream touches as little of it as possible: the desktop notes the
//! program writes (OSC 9, 99, 777), queued for the view to show at its own
//! pace, and DEC mode 2031 (see `color_scheme`). A `RemoteTerminal` and every
//! reader it spawns share one [`Signals`]; a reader drives it through three
//! calls on its [`Reader`].

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::EventListener as _;
use tty7_core::core::term_modes::TerminalModes;

use super::{DaemonMsg, EventProxy, OscNotifyScanner, RemoteTerminal};
use crate::terminal::TermSize;
use crate::terminal::color_scheme;

/// A desktop note: its title, if the program gave one, and its body.
pub(crate) type Note = (Option<String>, String);

/// Notes queued for a view that has not polled: a burst keeps only its
/// newest few. This bounds memory; [`Budget`] bounds the rate.
const MAX_QUEUED: usize = 3;
/// How many of a pane's notes reach the desktop per [`WINDOW`].
const PER_WINDOW: usize = 5;
const WINDOW: Duration = Duration::from_secs(10);

#[derive(Clone, Default)]
pub(super) struct Signals {
    notes: Arc<Mutex<VecDeque<Note>>>,
    /// Whether the program in the pane switched on DEC mode 2031.
    scheme_updates: Arc<AtomicBool>,
    /// Here rather than on the view because it is per pane and must outlive
    /// a relink, as `RemoteTerminal` does; readers never touch it.
    budget: Arc<Mutex<Budget>>,
}

impl Signals {
    /// A new reader's state. A new link replays the pane from scratch, 2031
    /// included (`term_modes` restores it ahead of the ring).
    pub(super) fn reader(&self) -> Reader {
        self.scheme_updates.store(false, Ordering::Relaxed);
        Reader {
            signals: self.clone(),
            modes: TerminalModes::new(),
            replay_pending: false,
        }
    }
}

pub(super) struct Reader {
    signals: Signals,
    modes: TerminalModes,
    /// A replay was read and has not ended yet.
    replay_pending: bool,
}

impl Reader {
    /// Live output: queue its notes, fold 2031, answer each `?996n`.
    pub(super) fn on_output(
        &mut self,
        bytes: &[u8],
        osc: &mut OscNotifyScanner,
        proxy: &EventProxy,
    ) {
        if let Ok(mut notes) = self.signals.notes.lock() {
            osc.feed(bytes, &mut *notes);
            let over = notes.len().saturating_sub(MAX_QUEUED);
            notes.drain(..over);
        }
        color_scheme::fold(&mut self.modes, bytes, &self.signals.scheme_updates);
        for _ in 0..self.modes.take_color_scheme_queries() {
            proxy.send_event(color_scheme::query_reply());
        }
    }

    /// Replayed output: fold 2031. A replayed `?996n` was answered long ago.
    pub(super) fn on_replay(&mut self, bytes: &[u8]) {
        color_scheme::fold(&mut self.modes, bytes, &self.signals.scheme_updates);
        self.modes.take_color_scheme_queries();
        self.replay_pending = true;
    }

    /// Every frame off the link. The daemon follows a replay's ring with its
    /// stored state (Cwd, Prompt, Agent, …), so the first frame that is not
    /// `Size` or `Snapshot` ends it — not running out of bytes, which a read
    /// split mid-replay does too. A replay that left 2031 on hears the scheme
    /// once: the theme may have flipped while no window was attached.
    ///
    /// Still early by one frame when the first live `Output` carries the
    /// program's exit and the shell's prompt: the report goes out before that
    /// frame folds 2031 off.
    pub(super) fn on_frame(&mut self, msg: &DaemonMsg, proxy: &EventProxy) {
        if matches!(msg, DaemonMsg::Size(_) | DaemonMsg::Snapshot(_)) {
            return;
        }
        if std::mem::take(&mut self.replay_pending)
            && self.signals.scheme_updates.load(Ordering::Relaxed)
        {
            proxy.send_event(color_scheme::query_reply());
        }
    }
}

/// Pane output is untrusted, and the queue cap alone still lets a flood
/// through at a few notes per poll. So at most [`PER_WINDOW`] of a pane's
/// notes are shown per [`WINDOW`]; the rest are dropped and said as one note
/// when the window turns over. A tumbling window: a burst straddling its edge
/// can show twice [`PER_WINDOW`] in quick succession.
#[derive(Default)]
struct Budget {
    since: Option<Instant>,
    shown: usize,
    dropped: usize,
}

impl Budget {
    /// Of `wanted` notes at `now`: how many to show, and how many dropped
    /// earlier are due to be said as one note (which takes a slot).
    fn admit(&mut self, now: Instant, wanted: usize) -> (usize, usize) {
        let mut due = 0;
        if self.since.is_none_or(|s| now.duration_since(s) >= WINDOW) {
            due = std::mem::take(&mut self.dropped);
            self.since = Some(now);
            self.shown = usize::from(due > 0);
        }
        let show = wanted.min(PER_WINDOW.saturating_sub(self.shown));
        self.shown += show;
        self.dropped += wanted - show;
        (show, due)
    }
}

impl RemoteTerminal {
    /// Desktop notes the program wrote, oldest first; the view decides
    /// whether each is shown.
    pub fn take_osc_notes(&self) -> Vec<Note> {
        self.fork
            .notes
            .lock()
            .map(|mut notes| notes.drain(..).collect())
            .unwrap_or_default()
    }

    /// Of `notes` about to be shown, the ones this pane's [`Budget`] lets
    /// through, and how many it dropped earlier are due to be said as one note.
    pub(crate) fn pace_notes(&self, mut notes: Vec<Note>, now: Instant) -> (Vec<Note>, usize) {
        let Ok(mut budget) = self.fork.budget.lock() else {
            return (notes, 0);
        };
        let (show, due) = budget.admit(now, notes.len());
        notes.truncate(show);
        (notes, due)
    }

    pub(in crate::terminal) fn color_scheme_updates(&self) -> bool {
        self.fork.scheme_updates.load(Ordering::Relaxed)
    }

    /// The grid and device-pixel cell this pane last asked for, once a
    /// layout has sized it; `None` while it still runs at its spawn size.
    pub(crate) fn laid_out_grid(&self) -> Option<(TermSize, u16, u16)> {
        let (w, h) = self.synced_cell;
        self.synced_size.then_some((self.size, w, h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::protocol::DaemonMsg;
    use crate::terminal::size::TermSize;

    /// Everything `term` queues within `wait`, stopping once `until` notes
    /// have come in.
    fn queued(term: &RemoteTerminal, until: usize, wait: Duration) -> Vec<String> {
        let deadline = Instant::now() + wait;
        let mut out = Vec::new();
        while out.len() < until && Instant::now() < deadline {
            out.extend(term.take_osc_notes().into_iter().map(|(_, body)| body));
            std::thread::sleep(Duration::from_millis(5));
        }
        out
    }

    fn pane() -> (RemoteTerminal, crate::daemon::transport::Stream) {
        crate::core::config::pin_test_config_dir();
        let (client, daemon) = crate::terminal::view::test_stream_pair();
        let term = RemoteTerminal::from_stream(client, TermSize::new(80, 24)).unwrap();
        (term, daemon)
    }

    fn flood(from: usize, to: usize) -> DaemonMsg {
        DaemonMsg::Output(
            (from..to)
                .flat_map(|n| format!("\x1b]9;note {n}\x07").into_bytes())
                .collect(),
        )
    }

    #[test]
    fn a_flood_between_two_polls_queues_only_its_newest_few() {
        let (term, mut daemon) = pane();
        flood(0, 40).encode(&mut daemon).unwrap();
        assert_eq!(
            queued(&term, 3, Duration::from_secs(5)),
            ["note 37", "note 38", "note 39"]
        );
        assert!(term.take_osc_notes().is_empty(), "the rest were dropped");
    }

    #[test]
    fn a_flood_over_many_polls_shows_a_few_per_window_and_one_note_for_the_rest() {
        let mut budget = Budget::default();
        let t0 = Instant::now();
        // Three notes per 300ms poll for ten seconds.
        let mut shown = 0;
        for poll in 0..33 {
            let (show, due) = budget.admit(t0 + Duration::from_millis(300 * poll), 3);
            shown += show;
            assert_eq!(due, 0);
        }
        assert_eq!(shown, PER_WINDOW);
        let (show, due) = budget.admit(t0 + WINDOW, 3);
        assert_eq!(due, 33 * 3 - PER_WINDOW, "said once, as one note");
        assert_eq!(show, 3);
        let (show, due) = budget.admit(t0 + WINDOW, 3);
        assert_eq!((show, due), (PER_WINDOW - 4, 0), "the note took a slot");
        let (_, due) = budget.admit(t0 + WINDOW * 2, 0);
        assert_eq!(
            due,
            3 - (PER_WINDOW - 4),
            "said even when the flood stopped"
        );
    }
}
