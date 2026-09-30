//! Typing a line into a new shell at its first prompt, so it is not echoed
//! above everything the shell prints while it starts (a banner, `fastfetch`)
//! and read only afterwards. Every typed launch and resume goes through
//! [`type_at_first_prompt`]; how long it waits is [`prompt_patience`].

use std::time::Duration;

use gpui::{App, Entity};

use crate::core::config::Config;
use crate::terminal::view::TerminalView;
use tty7_core::daemon::pane::integrates;

const PROMPT_POLL: Duration = Duration::from_millis(50);

/// Whether `view`'s shell, and so its agent session, runs on this machine:
/// not a remote workspace's pane, not SSH (native or typed), not WSL.
pub(crate) fn on_this_machine(view: &TerminalView) -> bool {
    view.pane_route().is_local() && view.ssh_spec().is_none() && view.remote_context().is_none()
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
    let local = on_this_machine(view);
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

/// Type `line` into `view`'s shell once it reports a prompt, or when its
/// patience runs out (a shell with no integration never reports one).
pub(crate) fn type_at_first_prompt(view: &Entity<TerminalView>, line: String, cx: &mut App) {
    let patience = prompt_patience(view, cx);
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

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use gpui::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::daemon::protocol::{ClientMsg, DaemonMsg};
    use crate::daemon::transport::Stream;
    use crate::terminal::view::quiet_test_pane;

    const NONE: Duration = Duration::from_millis(100);
    const SOME: Duration = Duration::from_secs(2);

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

    /// Whether the line was typed into the pane after `wait` of test time,
    /// reading for up to `read` of real time: short when none is expected,
    /// long enough for a slow CI runner when one is.
    fn typed_after(
        wait: Duration,
        read: Duration,
        vcx: &mut VisualTestContext,
        daemon: &mut Stream,
    ) -> bool {
        vcx.executor().advance_clock(wait);
        vcx.run_until_parked();
        daemon.set_read_timeout(Some(read)).unwrap();
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
        assert!(typed_after(Duration::ZERO, SOME, &mut vcx, &mut daemon));
    }

    #[gpui::test]
    fn the_line_waits_for_the_first_prompt(cx: &mut TestAppContext) {
        let (mut vcx, view, mut daemon) = pane(cx);
        type_it(&view, Duration::from_secs(30), &mut vcx);
        assert!(!typed_after(
            Duration::from_secs(1),
            NONE,
            &mut vcx,
            &mut daemon
        ));
        report_prompt(&view, &mut vcx, &mut daemon);
        assert!(typed_after(PROMPT_POLL, SOME, &mut vcx, &mut daemon));
    }

    #[gpui::test]
    fn a_shell_without_integration_gets_it_after_the_short_wait(cx: &mut TestAppContext) {
        let (mut vcx, view, mut daemon) = pane(cx);
        type_it(&view, Duration::from_secs(3), &mut vcx);
        assert!(!typed_after(
            Duration::from_millis(2900),
            NONE,
            &mut vcx,
            &mut daemon
        ));
        assert!(typed_after(
            Duration::from_millis(200),
            SOME,
            &mut vcx,
            &mut daemon
        ));
    }

    #[gpui::test]
    fn an_integrated_shell_that_never_prompts_gets_it_at_the_cap(cx: &mut TestAppContext) {
        let (mut vcx, view, mut daemon) = pane(cx);
        type_it(&view, Duration::from_secs(30), &mut vcx);
        assert!(!typed_after(
            Duration::from_secs(4),
            NONE,
            &mut vcx,
            &mut daemon
        ));
        assert!(typed_after(
            Duration::from_secs(27),
            SOME,
            &mut vcx,
            &mut daemon
        ));
    }
}
