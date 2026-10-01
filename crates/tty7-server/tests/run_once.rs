//! A pane spawned to run a command once (a dead pane's agent, resumed): the
//! shell runs it itself, with tty7's integration loaded, its own config dir
//! back in place, and the request out of the environment the command sees.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tty7_core::client::PaneClient;
use tty7_core::daemon::protocol::{ClientMsg, DaemonMsg, RestoreFrom, ShellSpec, WinSize};
use tty7_core::daemon::transport;

const WITHIN: Duration = Duration::from_secs(30);

struct Daemon {
    child: Child,
    dir: tempfile::TempDir,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A daemon whose shells start in an empty home, so no user rc file runs.
fn daemon() -> Daemon {
    let dir = tempfile::TempDir::new().unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_tty7-server"))
        .arg("--daemon")
        .arg("--config-dir")
        .arg(dir.path())
        .env("HOME", dir.path())
        .env("TTY7_DATA_DIR", dir.path())
        .env("TTY7_CONTROL_SOCK", dir.path().join("control.sock"))
        .env_remove("ZDOTDIR")
        .env_remove("TTY7_USER_ZDOTDIR")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start tty7-server --daemon");
    let daemon = Daemon { child, dir };
    let deadline = Instant::now() + WITHIN;
    while PaneClient::at(endpoint(daemon.dir.path()))
        .version()
        .is_err()
    {
        assert!(Instant::now() < deadline, "the daemon did not come up");
        std::thread::sleep(Duration::from_millis(50));
    }
    daemon
}

fn endpoint(dir: &Path) -> PathBuf {
    dir.join("daemon.sock")
}

/// Spawn `program` in place of a dead pane, asked to run `line` once, and read
/// what it prints until `marker` shows.
fn run_once(daemon: &Daemon, program: &str, line: &str, marker: &str) -> String {
    let mut stream = transport::connect_endpoint_at(&endpoint(daemon.dir.path())).unwrap();
    ClientMsg::Spawn {
        cwd: Some(daemon.dir.path().to_path_buf()),
        size: WinSize {
            cols: 200,
            rows: 30,
            cell_w: 8,
            cell_h: 16,
        },
        shell: Some(ShellSpec {
            program: program.into(),
            args: Vec::new(),
            args_are_tty7_defaults: false,
        }),
        owner: None,
        workspace: None,
        restore: Some(RestoreFrom {
            pane_id: 999_999,
            banner: None,
            run_once: Some(line.into()),
        }),
        allow_remote_clipboard_write: false,
    }
    .encode(&mut stream)
    .unwrap();
    stream.set_read_timeout(Some(WITHIN)).unwrap();
    let mut seen = Vec::new();
    let deadline = Instant::now() + WITHIN;
    while Instant::now() < deadline {
        match DaemonMsg::read(&mut stream) {
            Ok(DaemonMsg::Output(bytes) | DaemonMsg::Snapshot(bytes)) => {
                seen.extend_from_slice(&bytes);
                let text = String::from_utf8_lossy(&seen);
                if let Some(at) = text.find(marker) {
                    return text[at..].lines().next().unwrap_or_default().to_string();
                }
            }
            Ok(DaemonMsg::Exited { code }) => panic!("{program} exited ({code:?})"),
            Ok(_) => {}
            Err(e) => panic!("{program}: {e}; saw {:?}", String::from_utf8_lossy(&seen)),
        }
    }
    panic!(
        "{program} never ran it; saw {:?}",
        String::from_utf8_lossy(&seen)
    );
}

fn installed(program: &str) -> bool {
    Command::new(program)
        .arg("-c")
        .arg("exit 0")
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn zsh_runs_it_once_with_its_integration_and_its_own_config_dir() {
    let daemon = daemon();
    let line = run_once(
        &daemon,
        "/bin/zsh",
        "print -r -- RAN_$((40+2)) ZD=${ZDOTDIR:-home} INT=$TTY7_SHELL_INTEGRATION ENV=${TTY7_RUN_ONCE-unset}",
        "RAN_42",
    );
    assert!(line.contains("INT=1"), "integration loaded: {line}");
    assert!(
        line.contains("ENV=unset"),
        "the request is out of its environment: {line}"
    );
    assert!(
        !line.contains("tty7-zdotdir"),
        "ZDOTDIR is the user's again: {line}"
    );
}

#[test]
fn bash_runs_it_once_with_its_integration() {
    if !installed("bash") {
        return;
    }
    let daemon = daemon();
    let line = run_once(
        &daemon,
        "bash",
        "echo RAN_$((40+2)) INT=$TTY7_SHELL_INTEGRATION ENV=${TTY7_RUN_ONCE-unset}",
        "RAN_42",
    );
    assert!(line.contains("INT=1"), "integration loaded: {line}");
    assert!(line.contains("ENV=unset"), "{line}");
}

#[test]
fn a_plain_shell_runs_it_then_becomes_itself() {
    let daemon = daemon();
    let line = run_once(&daemon, "/bin/sh", "echo RAN_$((40+2)) SH=$0", "RAN_42");
    assert!(line.contains("SH=/bin/sh"), "{line}");
}
