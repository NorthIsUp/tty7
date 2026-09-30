//! DEC mode 2031, colour palette update notifications: a program that switches
//! it on (`CSI ? 2031 h`) is sent `CSI ? 997 ; 1 n` (dark) or `; 2 n` (light)
//! whenever the theme's background changes, and `CSI ? 996 n` asks for the
//! same report. Claude Code's `theme: auto` depends on it: it reads OSC 11 once
//! at startup and again only when a 997 arrives.
//! <https://contour-terminal.org/vt-extensions/color-palette-update-notifications/>

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use alacritty_terminal::event::Event as AlacEvent;
use alacritty_terminal::vte::ansi::Rgb;
use gpui::{App, Context};
use gpui_component::ActiveTheme;
use tty7_core::core::term_modes::{COLOR_SCHEME_UPDATES, TerminalModes};

use super::remote::RemoteTerminal;
use super::view::TerminalView;

/// The report for a background: dark below mid-grey luma, the way programs
/// classify an OSC 11 answer.
pub(super) fn report(bg: Rgb) -> String {
    let luma = 0.2126 * f32::from(bg.r) + 0.7152 * f32::from(bg.g) + 0.0722 * f32::from(bg.b);
    format!("\x1b[?997;{}n", if luma < 127.5 { 1 } else { 2 })
}

/// Fold pane output into `modes` and publish whether 2031 is on.
pub(super) fn fold(modes: &mut TerminalModes, bytes: &[u8], on: &AtomicBool) {
    modes.feed(bytes);
    on.store(modes.is_on(COLOR_SCHEME_UPDATES), Ordering::Relaxed);
}

/// The answer to a `CSI ? 996 n`. Routed as a background colour request so
/// the view answers it from the theme it is drawing now, and a replay drops it
/// like every other reply.
pub(super) fn query_reply() -> AlacEvent {
    AlacEvent::ColorRequest(257, Arc::new(report))
}

fn background(cx: &App) -> Rgb {
    super::palette::hsla_to_rgb(cx.theme().background)
}

/// Push a report to the pane each time the theme's background changes while
/// its program has 2031 on. Every path to a new theme (a preset pick, a
/// follow-system flip, a config reload) ends in the `Theme` global.
pub(super) fn watch(
    cx: &mut Context<TerminalView>,
    terminal: fn(&TerminalView) -> &RemoteTerminal,
) {
    let mut last = background(cx);
    cx.observe_global::<gpui_component::Theme>(move |view, cx| {
        let now = background(cx);
        if std::mem::replace(&mut last, now) != now && terminal(view).color_scheme_updates() {
            terminal(view).write(report(now).into_bytes());
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Config;
    use crate::daemon::protocol::{ClientMsg, DaemonMsg};
    use crate::terminal::size::TermSize;
    use gpui::TestAppContext;
    use std::io::Write as _;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn a_dark_background_reports_dark_and_a_light_one_light() {
        assert_eq!(report(Rgb { r: 0, g: 43, b: 54 }), "\x1b[?997;1n");
        assert_eq!(
            report(Rgb {
                r: 253,
                g: 246,
                b: 227
            }),
            "\x1b[?997;2n"
        );
    }

    struct Pane {
        daemon: crate::daemon::transport::Stream,
        input: mpsc::Receiver<Vec<u8>>,
    }

    fn pane(cx: &mut TestAppContext) -> Pane {
        let (client, daemon) = crate::terminal::view::test_stream_pair();
        let mut reader = daemon.try_clone().unwrap();
        let (tx, input) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(msg) = ClientMsg::read(&mut reader) {
                if let ClientMsg::Input(bytes) = msg {
                    let _ = tx.send(bytes);
                }
            }
        });
        cx.add_window(|window, cx| {
            let terminal = RemoteTerminal::from_stream(client, TermSize::new(80, 24)).unwrap();
            TerminalView::with_terminal(terminal, 1, window, cx)
        });
        Pane { daemon, input }
    }

    /// Everything the pane typed back within `wait`, stopping early once it
    /// contains `until`.
    fn typed(cx: &mut TestAppContext, pane: &Pane, until: &str, wait: Duration) -> String {
        let mut out = Vec::new();
        let end = Instant::now() + wait;
        while Instant::now() < end {
            cx.run_until_parked();
            while let Ok(bytes) = pane.input.try_recv() {
                out.extend(bytes);
            }
            if !until.is_empty() && String::from_utf8_lossy(&out).contains(until) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        String::from_utf8(out).unwrap()
    }

    fn set_background(cx: &mut TestAppContext, bg: gpui::Hsla) {
        cx.update(|cx| gpui_component::Theme::global_mut(cx).background = bg);
    }

    #[gpui::test]
    fn a_theme_change_reaches_only_the_panes_that_asked(cx: &mut TestAppContext) {
        crate::core::config::pin_test_config_dir();
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(Config::default());
        });
        set_background(cx, gpui::white());
        let mut asked = pane(cx);
        let plain = pane(cx);

        DaemonMsg::Output(b"\x1b[?2031h\x1b[?996n".to_vec())
            .encode(&mut asked.daemon)
            .unwrap();
        let wait = Duration::from_secs(5);
        assert_eq!(
            typed(cx, &asked, "997", wait),
            "\x1b[?997;2n",
            "996 answers the current scheme"
        );

        set_background(cx, gpui::black());
        assert_eq!(typed(cx, &asked, "997", wait), "\x1b[?997;1n");
        DaemonMsg::Output(b"\x1b]11;?\x07".to_vec())
            .encode(&mut asked.daemon)
            .unwrap();
        assert!(
            typed(cx, &asked, "rgb:", wait).contains("11;rgb:0000/0000/0000"),
            "OSC 11 answers with the background the flip left"
        );

        set_background(cx, gpui::white());
        assert_eq!(typed(cx, &asked, "997", wait), "\x1b[?997;2n");
        assert_eq!(
            typed(cx, &plain, "", Duration::from_millis(200)),
            "",
            "a pane that never set 2031 is not written to"
        );

        DaemonMsg::Output(b"\x1b[?2031l".to_vec())
            .encode(&mut asked.daemon)
            .unwrap();
        let _ = typed(cx, &asked, "", Duration::from_millis(100));
        set_background(cx, gpui::black());
        assert_eq!(
            typed(cx, &asked, "", Duration::from_millis(200)),
            "",
            "2031 switched off"
        );
    }

    /// A reattach replays the modes and then each ring segment as its own
    /// Snapshot. The theme may have flipped while nothing was attached, so a
    /// replay that leaves 2031 on is answered once, after its last frame.
    #[gpui::test]
    fn a_replay_that_leaves_2031_on_reports_the_scheme_once(cx: &mut TestAppContext) {
        crate::core::config::pin_test_config_dir();
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(Config::default());
        });
        set_background(cx, gpui::black());
        let mut replayed = pane(cx);
        // No live frame follows: an idle pane still hears once.
        let size = crate::daemon::protocol::WinSize {
            cols: 80,
            rows: 24,
            cell_w: 8,
            cell_h: 16,
        };
        let mut replay = Vec::new();
        for frame in [
            DaemonMsg::Snapshot(b"\x1b[?2031h".to_vec()),
            DaemonMsg::Size(size),
            DaemonMsg::Snapshot(b"first segment\r\n".to_vec()),
            DaemonMsg::Snapshot(b"second segment\r\n".to_vec()),
        ] {
            frame.encode(&mut replay).unwrap();
        }
        replayed.daemon.write_all(&replay).unwrap();
        assert_eq!(
            typed(cx, &replayed, "", Duration::from_millis(500)),
            "\x1b[?997;1n"
        );
    }
}
