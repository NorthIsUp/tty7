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
    daemon_with(mode, "", None)
}

/// `daemon`, with `mise_first` run by the fake mise before it answers, and
/// `key` as the user's own mise env cache key.
fn daemon_with(mode: &str, mise_first: &str, key: Option<&str>) -> Daemon {
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
        &format!(
            r#"{mise_first}
[ "$1" = activate ] && echo "export FAKE_MISE=$2 FAKE_KEY=$__MISE_ENV_CACHE_KEY""#
        ),
    );
    script(&bin.join("asdf"), "exit 0");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tty7-server"));
    match key {
        Some(key) => cmd.env("__MISE_ENV_CACHE_KEY", key),
        None => cmd.env_remove("__MISE_ENV_CACHE_KEY"),
    };
    let child = cmd
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
/// report of the environment prints, and how long the prompt took.
fn zsh_pane(daemon: &Daemon) -> (String, String, Duration) {
    let mut stream = transport::connect_endpoint_at(&endpoint(daemon.dir.path())).unwrap();
    let spawned = Instant::now();
    let mut prompt = Duration::ZERO;
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
                    prompt = spawned.elapsed();
                    seen.clear();
                    ClientMsg::Input(
                        b"print -r -- OUT_$((1+1)) MISE=${FAKE_MISE-no} KEY=${FAKE_KEY-no} PATH=$PATH\r".to_vec(),
                    )
                    .encode(&mut stream)
                    .unwrap();
                } else if let Some(at) = text.find("OUT_2") {
                    let line = text[at..].lines().next().unwrap_or_default().to_string();
                    return (startup.unwrap_or_default(), line, prompt);
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
    let (startup, line, _) = zsh_pane(&daemon("user"));
    assert!(startup.contains("USER_RC_RAN"), "{startup:?}");
    assert!(line.contains("MISE=no"), "{line}");
}

#[test]
fn no_rcs_skips_the_users_zshrc_but_keeps_the_prompt_marks() {
    let (startup, line, _) = zsh_pane(&daemon("no_rcs"));
    assert!(!startup.contains("USER_RC_RAN"), "{startup:?}");
    assert!(startup.contains(PROMPT_MARK));
    assert!(line.contains("MISE=no"), "{line}");
}

#[test]
fn no_rcs_envlibs_activates_mise_and_not_asdf() {
    let (startup, line, _) = zsh_pane(&daemon("no_rcs_envlibs"));
    assert!(!startup.contains("USER_RC_RAN"), "{startup:?}");
    assert!(line.contains("MISE=zsh"), "{line}");
    assert!(!line.contains(".asdf/shims"), "{line}");
}

#[test]
fn a_slow_mise_does_not_hold_up_the_first_prompt() {
    let d = daemon_with("no_rcs_envlibs", "sleep 2", None);
    let (_, line, prompt) = zsh_pane(&d);
    assert!(
        prompt < Duration::from_secs(1),
        "the prompt waited {prompt:?} for mise"
    );
    // The command went in at the prompt, before mise answered, and still saw it.
    assert!(line.contains("MISE=zsh"), "{line}");
}

fn key(line: &str) -> &str {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix("KEY="))
        .unwrap_or_default()
}

#[test]
fn panes_of_one_daemon_share_a_mise_cache_key() {
    let d = daemon("no_rcs_envlibs");
    let (_, first, _) = zsh_pane(&d);
    let (_, second, _) = zsh_pane(&d);
    assert_eq!(key(&first).len(), 44, "{first}");
    assert_eq!(key(&first), key(&second));
}

#[test]
fn the_users_own_mise_cache_key_is_kept() {
    let user = "dXNlcnMgb3duIGtleSwgbm90IG1pbnRlZCBieSB0dHk3IQ==";
    let (_, line, _) = zsh_pane(&daemon_with("no_rcs_envlibs", "", Some(user)));
    assert_eq!(key(&line), user);
}
