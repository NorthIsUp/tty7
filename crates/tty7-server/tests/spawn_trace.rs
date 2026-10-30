//! `spawn-trace`: a real zsh pane's daemon timeline lands in `tty7.log`, each
//! stage once and in order, through its first prompt and first command.
#![cfg(unix)]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tty7_core::client::PaneClient;
use tty7_core::daemon::protocol::{ClientMsg, DaemonMsg, ShellSpec, WinSize};
use tty7_core::daemon::transport;

const WITHIN: Duration = Duration::from_secs(30);
const STAGES: [&str; 5] = [
    "spawn_requested",
    "child_started",
    "first_output",
    "prompt",
    "command",
];

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn stages(log: &str, pane: u64) -> Vec<String> {
    let tag = format!("spawn-trace pane={pane} stage=");
    log.lines()
        .filter_map(|l| l.split_once(&tag))
        .map(|(_, rest)| rest.split(' ').next().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn a_zsh_pane_logs_every_daemon_stage_once_in_order() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path();
    std::fs::write(home.join(".zshrc"), "").unwrap();
    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_tty7-server"))
            .arg("--daemon")
            .arg("--config-dir")
            .arg(home)
            .env("HOME", home)
            .env("TTY7_LOG", "info")
            .env("TTY7_DATA_DIR", home)
            .env("TTY7_CONTROL_SOCK", home.join("control.sock"))
            .env_remove("ZDOTDIR")
            .env_remove("TTY7_USER_ZDOTDIR")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start tty7-server --daemon"),
    );
    let endpoint = home.join("daemon.sock");
    let deadline = Instant::now() + WITHIN;
    while PaneClient::at(endpoint.clone()).version().is_err() {
        assert!(Instant::now() < deadline, "the daemon did not come up");
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut stream = transport::connect_endpoint_at(&endpoint).unwrap();
    ClientMsg::Spawn {
        cwd: Some(home.to_path_buf()),
        size: WinSize {
            cols: 120,
            rows: 30,
            cell_w: 8,
            cell_h: 16,
        },
        shell: Some(ShellSpec {
            program: "/bin/zsh".into(),
            args: Vec::new(),
            args_are_tty7_defaults: false,
        }),
        owner: None,
        workspace: None,
        restore: None,
        allow_remote_clipboard_write: false,
    }
    .encode(&mut stream)
    .unwrap();
    stream.set_read_timeout(Some(WITHIN)).unwrap();

    let log = home.join("tty7.log");
    let mut pane = None;
    let mut seen = Vec::new();
    let mut typed = false;
    while Instant::now() < deadline {
        match DaemonMsg::read(&mut stream) {
            Ok(DaemonMsg::Spawned { pane_id }) => pane = Some(pane_id),
            Ok(DaemonMsg::Output(bytes) | DaemonMsg::Snapshot(bytes)) => {
                seen.extend_from_slice(&bytes);
                if !typed && String::from_utf8_lossy(&seen).contains("\x1b]133;A") {
                    typed = true;
                    ClientMsg::Input(b"true\r".to_vec())
                        .encode(&mut stream)
                        .unwrap();
                }
            }
            Ok(DaemonMsg::Exited { code }) => panic!("zsh exited ({code:?})"),
            Ok(_) => {}
            Err(e) => panic!("{e}; saw {:?}", String::from_utf8_lossy(&seen)),
        }
        let pane = pane.expect("Spawned comes first");
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let got = stages(&text, pane);
        if got.last().map(String::as_str) == Some("command") {
            assert_eq!(got, STAGES, "{text}");
            return;
        }
    }
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    panic!("no command stage; log:\n{text}");
}

#[test]
fn stage_names_are_read_off_the_line() {
    let log = "2026-01-01T00:00:00.000Z INFO  server pid=1 [x] spawn-trace pane=3 stage=prompt ms=12.5\n\
               2026-01-01T00:00:00.000Z INFO  server pid=1 [x] spawn-trace pane=31 stage=command ms=1.0";
    assert_eq!(stages(log, 3), ["prompt"]);
}
