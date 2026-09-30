//! Lowering a pane's shell priority (`nice` in the config), so agents and
//! builds typed into it yield the CPU to the GUI and to each other. Children
//! inherit the value, so only the shell itself needs setting.

#[cfg(unix)]
use crate::core::config::Config;

/// The value to hand `setpriority`, or `None` when there is nothing to do.
/// Negative values would need root to raise priority, so they clamp to 0 (off).
pub(crate) fn clamp(n: i32) -> Option<i32> {
    match n.clamp(0, 19) {
        0 => None,
        n => Some(n),
    }
}

/// Applies the configured nice to a freshly spawned shell. A failure is
/// logged, never fatal: a pane at normal priority beats no pane.
#[cfg(unix)]
pub(crate) fn apply(pid: u32) {
    let Some(n) = clamp(Config::load().fork.nice) else {
        return;
    };
    apply_n(pid, n);
    // A shell spawned in the daemon's first moments (the restored panes) was
    // measured back at 0 a second later on macOS, while later spawns kept
    // the value; the reset is outside tty7. Looking again settles it.
    // ponytail: two fixed rechecks, not a watch; a reset after 5s would stick.
    std::thread::spawn(move || {
        for wait in [1, 4] {
            std::thread::sleep(std::time::Duration::from_secs(wait));
            // SAFETY: plain integers in, a plain integer out.
            if unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t) } < n {
                apply_n(pid, n);
            }
        }
    });
}

#[cfg(not(unix))]
pub(crate) fn apply(_pid: u32) {}

#[cfg(unix)]
fn apply_n(pid: u32, n: i32) {
    // SAFETY: setpriority takes plain integers and touches no memory of ours.
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, n) } != 0 {
        log::warn!(
            "could not nice pane shell {pid} to {n}: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_turns_zero_and_negatives_off_and_caps_at_nineteen() {
        for (input, want) in [(0, None), (5, Some(5)), (-3, None), (40, Some(19))] {
            assert_eq!(clamp(input), want, "clamp({input})");
        }
    }

    #[cfg(unix)]
    #[test]
    fn apply_n_sets_the_priority_of_a_running_process() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = child.id();
        apply_n(pid, 7);
        let got = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t) };
        child.kill().ok();
        child.wait().ok();
        assert_eq!(got, 7);
    }
}
