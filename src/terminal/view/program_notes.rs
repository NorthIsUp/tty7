//! Which notices about a pane reach the desktop, and the ones its program
//! writes itself (OSC 9, 99, 777), such as Claude Code's with its
//! Notifications setting on `ghostty`, `kitty` or `iterm2`. The agent's
//! Waiting mark on the tab is the daemon's doing.
//!
//! One rule for all of them, command-finish and agent notices included:
//! `Unfocused` holds a notice back only while the reader is looking at this
//! very pane, so another tab in the key window still hears about it.

use std::time::Instant;

use gpui::{Context, Window};

use super::TerminalView;
use crate::core::config::Config;
use crate::ui::i18n::{L10nKey, t};

/// Whether a notice about `view` reaches the desktop now.
pub(super) fn allowed(view: &TerminalView, window: &Window, cx: &Context<TerminalView>) -> bool {
    let watched = window.is_window_active() && view.focus_handle.is_focused(window);
    cx.global::<Config>()
        .notify_on_command_finish
        .allows(watched)
}

/// Shows what the program wrote since the last poll, at most a few per pane
/// every few seconds (pane output is untrusted), with one note saying the
/// rest were not shown.
/// Drained even from a pane whose shell has exited, so its last words are
/// not lost.
pub(super) fn show(view: &TerminalView, window: &Window, cx: &mut Context<TerminalView>) {
    let notes = view.terminal.take_osc_notes();
    // An agent reporting through tty7's hooks gets its notices from
    // `poll_agent_status` under the same rule; its own copy would repeat them.
    let show = !view.terminal.agent_session().is_some_and(|s| s.rich) && allowed(view, window, cx);
    if !notes.is_empty() {
        log::debug!(
            "{} program notification(s) {}",
            notes.len(),
            if show { "shown" } else { "held back" }
        );
    }
    // Asked on every poll, so the rest are said once a flood stops too, and
    // while held back, so a stale count never surfaces minutes later.
    let (notes, dropped) = view
        .terminal
        .pace_notes(if show { notes } else { Vec::new() }, Instant::now());
    if !show {
        return;
    }
    let agent = view.terminal.foreground_agent().map(|a| a.display_name());
    if dropped > 0 {
        view.notify_pane(agent, &t(L10nKey::ProgramNotesDropped), cx);
    }
    for (title, body) in notes {
        match title {
            Some(title) => super::super::remote::notify_desktop_for_pane(
                Some(&title),
                &body,
                Some(cx.entity_id()),
            ),
            None => view.notify_pane(agent, &body, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notification_is_held_back_only_from_the_pane_being_watched() {
        use crate::core::config::NotifyMode::*;
        // (mode, window active, pane focused) -> shown; every notice about a
        // pane goes through this.
        let cases = [
            (Unfocused, true, true, false),
            // Another tab in the key window still hears about this one.
            (Unfocused, true, false, true),
            (Unfocused, false, true, true),
            (Unfocused, false, false, true),
            (Always, true, true, true),
            (Always, false, false, true),
            (Never, false, false, false),
            (Never, true, false, false),
        ];
        for (mode, active, focused, shown) in cases {
            assert_eq!(
                mode.allows(active && focused),
                shown,
                "{mode:?} active={active} focused={focused}"
            );
        }
    }
}
