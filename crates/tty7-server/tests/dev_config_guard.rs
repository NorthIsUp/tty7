//! A dev build will not serve the default config dir unless told to. Each
//! case runs with a scratch `HOME`, so the default dir here is the test's own
//! — it is the dir a forgotten isolation would land in, not the user's.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tty7_core::client::PaneClient;
use tty7_core::daemon::protocol::{DaemonMsg, ShellSpec, WinSize};

const EXE: &str = env!("CARGO_BIN_EXE_tty7-server");
const OPT_IN: &str = "TTY7_DEV_USE_REAL_CONFIG";

/// A daemon on `exe` serving `home`'s default config dir.
fn daemon(exe: &Path, home: &Path, opt_in: bool) -> Command {
    let mut cmd = Command::new(exe);
    cmd.arg("--daemon")
        .env("HOME", home)
        .env_remove("TTY7_CONFIG_DIR")
        .env_remove("TTY7_DATA_DIR")
        .env_remove("TTY7_CONTROL_SOCK")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove(OPT_IN)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if opt_in {
        cmd.env(OPT_IN, "1");
    }
    cmd
}

fn panes(home: &Path) -> PaneClient {
    PaneClient::at(home.join(".config/tty7/daemon.sock"))
}

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn await_ready(home: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while panes(home).version().is_err() {
        assert!(Instant::now() < deadline, "the daemon never answered");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_dev_build_refuses_the_default_config_dir() {
    let home = tempfile::TempDir::new().unwrap();
    let out = daemon(Path::new(EXE), home.path(), false).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(OPT_IN),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn an_opted_in_dev_build_serves_it() {
    let home = tempfile::TempDir::new().unwrap();
    let _daemon = Killed(daemon(Path::new(EXE), home.path(), true).spawn().unwrap());
    await_ready(home.path());
}

/// An installed build serving the default dir will not hand its panes to a
/// dev build that would refuse it: it keeps serving, and the shell lives.
#[test]
fn a_handoff_to_a_refusing_dev_build_leaves_the_panes_where_they_are() {
    let home = tempfile::TempDir::new().unwrap();
    // Outside any cargo target dir, so this copy is not a dev build.
    let bin = tempfile::TempDir::new().unwrap();
    let installed: PathBuf = bin.path().join("tty7-server");
    std::fs::copy(EXE, &installed).unwrap();
    let _daemon = Killed(daemon(&installed, home.path(), false).spawn().unwrap());
    await_ready(home.path());
    let client = panes(home.path());
    let before = client.version().unwrap().instance;

    let size = WinSize {
        cols: 80,
        rows: 24,
        cell_w: 8,
        cell_h: 16,
    };
    let shell = ShellSpec {
        program: "/bin/sh".into(),
        args: Vec::new(),
        args_are_tty7_defaults: false,
    };
    let mut session = client.spawn(None, size, Some(shell), None, None).unwrap();
    session
        .set_recv_timeout(Some(Duration::from_secs(30)))
        .unwrap();

    let err = client
        .hand_off(Path::new(EXE))
        .expect_err("the dev build would refuse this dir");
    assert!(err.to_string().contains(OPT_IN), "{err}");
    assert_eq!(
        client.version().unwrap().instance,
        before,
        "nothing replaced"
    );

    session.input(b"echo tty7_still_$((40+2))\r").unwrap();
    let mut seen = Vec::new();
    while !seen.windows(13).any(|w| w == b"tty7_still_42") {
        match session.recv().expect("the pane is still served") {
            DaemonMsg::Output(b) | DaemonMsg::Snapshot(b) => seen.extend_from_slice(&b),
            DaemonMsg::Exited { code } => panic!("the shell exited ({code:?})"),
            _ => {}
        }
    }
    session.kill().unwrap();
}
