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
            recheck(pid, n, std::process::id());
        }
    });
}

#[cfg(not(unix))]
pub(crate) fn apply(_pid: u32) {}

/// Put `pid` back at `n` if something reset it, but only while it is still
/// `parent`'s child: the shell may have exited since, and its pid gone to a
/// process that is none of ours.
#[cfg(unix)]
fn recheck(pid: u32, n: i32, parent: u32) {
    if parent_of(pid) != Some(parent) {
        return;
    }
    // SAFETY: plain integers in, a plain integer out.
    if unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t) } < n {
        apply_n(pid, n);
    }
}

#[cfg(target_os = "macos")]
fn parent_of(pid: u32) -> Option<u32> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a zeroed `proc_bsdinfo` of exactly `size` bytes.
    let got = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    (got == size).then_some(info.pbi_ppid)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = &stat[stat.rfind(')')? + 1..];
    after_name.split_whitespace().nth(1)?.parse().ok()
}

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

    /// A pid that is not the daemon's child is left alone, whatever its
    /// priority: after the shell exits, its number can belong to anyone.
    #[cfg(unix)]
    #[test]
    fn a_recheck_touches_only_the_parents_own_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = child.id();
        let prio = || unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t) };
        assert_eq!(parent_of(pid), Some(std::process::id()));
        let before = prio();
        recheck(pid, 19, std::process::id() + 1);
        let stranger = prio();
        recheck(pid, 19, std::process::id());
        let own = prio();
        child.kill().ok();
        child.wait().ok();
        assert_eq!(stranger, before, "not our child: left alone");
        assert_eq!(own, 19);
    }
}
