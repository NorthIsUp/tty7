//! `shell_startup`: a zsh pane that skips the user's `.zshrc` still loads
//! tty7's integration, and with env tools activates mise alone.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tty7_core::client::PaneClient;
use tty7_core::daemon::protocol::{ClientMsg, DaemonMsg, ShellSpec, WinSize};
use tty7_core::daemon::transport;

const WITHIN: Duration = Duration::from_secs(30);
const PROMPT_MARK: &str = "\x1b]133;A";

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

fn script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A daemon whose home has a `.zshrc` that prints a marker, with a fake mise
/// and asdf first on its PATH.
fn daemon(mode: &str) -> Daemon {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path();
    std::fs::write(home.join(".zshrc"), "echo USER_RC_RAN\n").unwrap();
    std::fs::write(
        home.join("config.json"),
        format!(r#"{{"shell_startup": "{mode}"}}"#),
    )
    .unwrap();
    let bin = home.join("bin");
    std::fs::create_dir(&bin).unwrap();
    script(
        &bin.join("mise"),
        r#"[ "$1" = activate ] && echo "export FAKE_MISE=$2""#,
    );
    script(&bin.join("asdf"), "exit 0");
    let child = Command::new(env!("CARGO_BIN_EXE_tty7-server"))
        .arg("--daemon")
        .arg("--config-dir")
        .arg(home)
        .env("HOME", home)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("TTY7_DATA_DIR", home)
        .env("TTY7_CONTROL_SOCK", home.join("control.sock"))
        .env_remove("ZDOTDIR")
        .env_remove("TTY7_USER_ZDOTDIR")
        .env_remove("ASDF_DATA_DIR")
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

/// Everything a zsh pane prints through its first prompt, then the line its
/// report of the environment prints.
fn zsh_pane(daemon: &Daemon) -> (String, String) {
    let mut stream = transport::connect_endpoint_at(&endpoint(daemon.dir.path())).unwrap();
    ClientMsg::Spawn {
        cwd: Some(daemon.dir.path().to_path_buf()),
        size: WinSize {
            cols: 400,
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
    let mut seen = Vec::new();
    let mut startup = None;
    let deadline = Instant::now() + WITHIN;
    while Instant::now() < deadline {
        match DaemonMsg::read(&mut stream) {
            Ok(DaemonMsg::Output(bytes) | DaemonMsg::Snapshot(bytes)) => {
                seen.extend_from_slice(&bytes);
                let text = String::from_utf8_lossy(&seen).into_owned();
                if startup.is_none() && text.contains(PROMPT_MARK) {
                    startup = Some(text);
                    seen.clear();
                    ClientMsg::Input(
                        b"print -r -- OUT_$((1+1)) MISE=${FAKE_MISE-no} PATH=$PATH\r".to_vec(),
                    )
                    .encode(&mut stream)
                    .unwrap();
                } else if let Some(at) = text.find("OUT_2") {
                    let line = text[at..].lines().next().unwrap_or_default().to_string();
                    return (startup.unwrap_or_default(), line);
                }
            }
            Ok(DaemonMsg::Exited { code }) => panic!("zsh exited ({code:?})"),
            Ok(_) => {}
            Err(e) => panic!("{e}; saw {:?}", String::from_utf8_lossy(&seen)),
        }
    }
    panic!("no answer; saw {:?}", String::from_utf8_lossy(&seen));
}

#[test]
fn user_mode_reads_the_users_zshrc() {
    let (startup, line) = zsh_pane(&daemon("user"));
    assert!(startup.contains("USER_RC_RAN"), "{startup:?}");
    assert!(line.contains("MISE=no"), "{line}");
}

#[test]
fn no_rcs_skips_the_users_zshrc_but_keeps_the_prompt_marks() {
    let (startup, line) = zsh_pane(&daemon("no_rcs"));
    assert!(!startup.contains("USER_RC_RAN"), "{startup:?}");
    assert!(startup.contains(PROMPT_MARK));
    assert!(line.contains("MISE=no"), "{line}");
}

#[test]
fn no_rcs_envlibs_activates_mise_and_not_asdf() {
    let (startup, line) = zsh_pane(&daemon("no_rcs_envlibs"));
    assert!(!startup.contains("USER_RC_RAN"), "{startup:?}");
    assert!(line.contains("MISE=zsh"), "{line}");
    assert!(!line.contains(".asdf/shims"), "{line}");
}
