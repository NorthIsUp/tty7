//! A dev build (a binary under a cargo target dir) refuses to serve the
//! user's real config dir: a test or dev run that forgot to isolate itself
//! would otherwise read and rewrite their live `machine.json`. Compared
//! against the default dir, so an inherited `TTY7_CONFIG_DIR` naming the real
//! one still trips it. `mise run run` opts in with [`OPT_IN`].
//!
//! Checked before anything is taken: a fresh start exits up front, and a
//! server refuses to hand its panes to a dev build that would be refused. A
//! server that adopted panes anyway (handed over by an older build) serves
//! panes only, never exits, so no shell is hung up.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const OPT_IN: &str = "TTY7_DEV_USE_REAL_CONFIG";

/// Whether this process was told to serve the real dir. Read once, then
/// taken out of the environment so it never reaches a pane; `run_daemon`
/// reads it first thing, before any thread exists.
pub fn opted_in() -> bool {
    static OPTED_IN: OnceLock<bool> = OnceLock::new();
    *OPTED_IN.get_or_init(|| {
        let yes = std::env::var_os(OPT_IN).is_some_and(|v| v == "1");
        // SAFETY: first called before the process starts any thread.
        unsafe { std::env::remove_var(OPT_IN) };
        yes
    })
}

fn refusal_for(dev: bool) -> Option<String> {
    let config = crate::core::config::config_dir_path();
    let default = crate::core::config::default_config_dir();
    refuses(dev, opted_in(), config.as_deref(), default.as_deref()).then(|| {
        format!(
            "a dev build will not serve the real config dir {}; \
             pass --config-dir <scratch>, or set {OPT_IN}=1 to mean it",
            config.unwrap_or_default().display()
        )
    })
}

/// Why this process must not serve, if it must not.
pub fn refusal() -> Option<String> {
    refusal_for(crate::core::dev_build::running_dev_build())
}

/// Exit before taking anything when this process must not serve.
pub fn enforce() {
    if let Some(why) = refusal() {
        eprintln!("tty7-server: {why}");
        std::process::exit(2);
    }
}

/// Why panes must not be handed to `exe`, checked while this server still
/// holds them. An allowed handoff carries this process's opt-in on the exec.
pub fn handoff_refusal(exe: &Path) -> Option<String> {
    refusal_for(crate::core::dev_build::is_dev_build(exe))
}

fn refuses(dev: bool, opted_in: bool, config: Option<&Path>, default: Option<&Path>) -> bool {
    dev && !opted_in && config.is_some() && config.map(real) == default.map(real)
}

fn real(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_build_refuses_only_the_default_dir_unless_opted_in() {
        let (real_dir, other) = (
            Some(Path::new("/h/.config/tty7")),
            Some(Path::new("/tmp/x")),
        );
        assert!(refuses(true, false, real_dir, real_dir));
        assert!(!refuses(true, true, real_dir, real_dir), "opted in");
        assert!(
            !refuses(false, false, real_dir, real_dir),
            "an installed build"
        );
        assert!(!refuses(true, false, other, real_dir), "a scratch dir");
    }
}
