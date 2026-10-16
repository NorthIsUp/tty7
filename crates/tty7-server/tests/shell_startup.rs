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
    // Logs each call as `<parent pid> <args>`. Its zsh activation ends the way
    // mise's does, `_mise_hook --force` unless `FAKE_FORCE_LINE` says otherwise,
    // then creates `loaded`: after that line, so only the foreground's eval does.
    let log = home.join("mise.log");
    script(
        &bin.join("mise"),
        &format!(
            r#"{mise_first}
echo "$PPID $*" >> '{log}'
case $1 in
activate)
  echo "export FAKE_MISE=$2 FAKE_KEY=$__MISE_ENV_CACHE_KEY"
  [ "$2" = zsh ] && echo "_mise_hook() {{ eval \"\$('$0' hook-env -s zsh \"\$@\")\"; }}
_mise_hook_chpwd() {{ _mise_hook --reason chpwd; }}
autoload -Uz add-zsh-hook
_mise_hook_precmd() {{ :; }}
add-zsh-hook chpwd _mise_hook_chpwd
add-zsh-hook precmd _mise_hook_precmd
${{FAKE_FORCE_LINE-_mise_hook --force}}
: >| '{loaded}'" ;;
hook-env)
  case $* in *--force*) echo FAKE_HOOK_WARNING >&2 ;; esac
  echo "export FAKE_HOOK='$*'" ;;
esac"#,
            log = log.display(),
            loaded = home.join("loaded").display(),
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

const REPORT: &str = "print -r -- OUT_$((1+1)) MISE=${FAKE_MISE-no} KEY=${FAKE_KEY-no} \\
    LAST=${precmd_functions[-1]} PATH=$PATH";

/// Everything a zsh pane prints through its first prompt, then the line its
/// report of the environment prints.
fn zsh_pane(daemon: &Daemon) -> (String, String) {
    let (startup, line, _) = zsh_pane_running(daemon, REPORT, || {}, || {});
    (startup, line)
}

/// `zsh_pane`, typing `command` at the first prompt between `before` and
/// `after`; its output must start with `OUT_2`. Also returns everything printed
/// after the prompt.
fn zsh_pane_running(
    daemon: &Daemon,
    command: &str,
    before: impl FnOnce(),
    after: impl FnOnce(),
) -> (String, String, String) {
    let mut pane = Pane::spawn(daemon, "/bin/zsh", daemon.dir.path());
    let startup = pane.until(PROMPT_MARK);
    before();
    pane.type_line(command);
    after();
    let text = pane.until_line("OUT_2");
    let at = text.find("OUT_2").unwrap();
    let line = text[at..].lines().next().unwrap_or_default().to_string();
    (startup, line, text)
}

/// A pane of `daemon` running `shell` in `cwd`.
struct Pane {
    stream: transport::Stream,
    seen: String,
}

impl Pane {
    fn spawn(daemon: &Daemon, shell: &str, cwd: &Path) -> Pane {
        let mut stream = transport::connect_endpoint_at(&endpoint(daemon.dir.path())).unwrap();
        ClientMsg::Spawn {
            cwd: Some(cwd.to_path_buf()),
            size: WinSize {
                cols: 400,
                rows: 30,
                cell_w: 8,
                cell_h: 16,
            },
            shell: Some(ShellSpec {
                program: shell.into(),
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
        Pane {
            stream,
            seen: String::new(),
        }
    }

    /// Everything printed since the last call, through the first `pat`.
    fn until(&mut self, pat: &str) -> String {
        let deadline = Instant::now() + WITHIN;
        let mut bytes = std::mem::take(&mut self.seen).into_bytes();
        while Instant::now() < deadline {
            let text = String::from_utf8_lossy(&bytes);
            if let Some(at) = text.find(pat) {
                let end = at + pat.len();
                self.seen = text[end..].to_string();
                return text[..end].to_string();
            }
            match DaemonMsg::read(&mut self.stream) {
                Ok(DaemonMsg::Output(b) | DaemonMsg::Snapshot(b)) => bytes.extend_from_slice(&b),
                Ok(DaemonMsg::Exited { code }) => panic!("the shell exited ({code:?})"),
                Ok(_) => {}
                Err(e) => panic!("{e}; saw {:?}", String::from_utf8_lossy(&bytes)),
            }
        }
        panic!("no {pat:?}; saw {:?}", String::from_utf8_lossy(&bytes));
    }

    /// `until` the line holding `pat` has ended.
    fn until_line(&mut self, pat: &str) -> String {
        let mut text = self.until(pat);
        text.push_str(&self.until("\n"));
        text
    }

    fn type_line(&mut self, line: &str) {
        self.send(&format!("{line}\r"));
    }

    fn send(&mut self, keys: &str) {
        ClientMsg::Input(keys.as_bytes().to_vec())
            .encode(&mut self.stream)
            .unwrap();
    }
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

const GATE: &str = r#"while [ ! -e "$HOME/go" ]; do sleep 0.05; done"#;

#[test]
fn a_command_entered_before_mise_answers_waits_for_it() {
    // The prompt comes up while mise is still held at the gate.
    let d = daemon_with("no_rcs_envlibs", GATE, None);
    let go = d.dir.path().join("go");
    let (_, line, _) = zsh_pane_running(&d, REPORT, || {}, || std::fs::write(&go, "").unwrap());
    assert!(line.contains("MISE=zsh"), "{line}");
    assert!(line.contains("LAST=__tty7_precmd "), "{line}");
}

#[test]
fn mise_loads_at_the_idle_prompt_and_the_prompt_mark_hook_stays_last() {
    let d = daemon_with("no_rcs_envlibs", GATE, None);
    let home = d.dir.path().to_path_buf();
    let (_, line, _) = zsh_pane_running(
        &d,
        REPORT,
        || {
            std::fs::write(home.join("go"), "").unwrap();
            let deadline = Instant::now() + WITHIN;
            while !home.join("loaded").exists() {
                assert!(Instant::now() < deadline, "mise never loaded at the prompt");
                std::thread::sleep(Duration::from_millis(20));
            }
        },
        || {},
    );
    assert!(line.contains("MISE=zsh"), "{line}");
    assert!(line.contains("LAST=__tty7_precmd "), "{line}");
}

fn key(line: &str) -> &str {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix("KEY="))
        .unwrap_or_default()
}

#[test]
fn panes_of_one_daemon_share_a_mise_cache_key() {
    let d = daemon("no_rcs_envlibs");
    let (_, first) = zsh_pane(&d);
    let (_, second) = zsh_pane(&d);
    assert_eq!(key(&first).len(), 44, "{first}");
    assert_eq!(key(&first), key(&second));
}

#[test]
fn the_users_own_mise_cache_key_is_kept() {
    let user = "dXNlcnMgb3duIGtleSwgbm90IG1pbnRlZCBieSB0dHk3IQ==";
    let (_, line) = zsh_pane(&daemon_with("no_rcs_envlibs", "", Some(user)));
    assert_eq!(key(&line), user);
}

/// What an immediate command, then a `cd`, see of the fake mise's hook-env,
/// with the pane's pid, the fake mise's call log and what the pane printed.
fn hook_pane(mise_first: &str) -> (String, String, String) {
    let d = daemon_with("no_rcs_envlibs", mise_first, None);
    let (_, line, seen) = zsh_pane_running(
        &d,
        "H1=$FAKE_HOOK; mkdir -p sub && cd sub; \\
         print -r -- OUT_$((1+1)) PID=$$ H1=${H1// /_} H2=${FAKE_HOOK// /_}",
        || {},
        || {},
    );
    let log = std::fs::read_to_string(d.dir.path().join("mise.log")).unwrap();
    (line, log, seen)
}

fn field<'a>(line: &'a str, name: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(name)?.strip_prefix('='))
        .unwrap_or_default()
}

/// The fake mise's hook-env calls, as `(parent pid, args)`.
fn hook_calls(log: &str) -> Vec<(&str, &str)> {
    log.lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(_, args)| args.starts_with("hook-env"))
        .collect()
}

#[test]
fn mises_initial_hook_env_runs_off_the_foreground() {
    let (line, log, seen) = hook_pane("");
    let pid = field(&line, "PID");
    assert_eq!(
        field(&line, "H1"),
        format!("hook-env_--shell-pid_{pid}_--force_-s_zsh"),
        "{line}"
    );
    assert_eq!(
        field(&line, "H2"),
        "hook-env_-s_zsh_--reason_chpwd",
        "{line}"
    );
    let calls = hook_calls(&log);
    let forced: Vec<_> = calls
        .iter()
        .filter(|(_, a)| a.contains("--force"))
        .collect();
    assert_eq!(forced.len(), 1, "{log}");
    assert_ne!(forced[0].0, pid, "{log}");
    let chpwd: Vec<_> = calls
        .iter()
        .filter(|(_, a)| a.contains("--reason chpwd"))
        .collect();
    assert_eq!(chpwd, [&(pid, "hook-env -s zsh --reason chpwd")], "{log}");
    assert!(seen.contains("FAKE_HOOK_WARNING"), "{seen:?}");
}

#[test]
fn without_mises_force_line_the_activation_runs_as_given() {
    let (line, log, _) = hook_pane("FAKE_FORCE_LINE=_mise_hook");
    let pid = field(&line, "PID");
    assert_eq!(field(&line, "H1"), "hook-env_-s_zsh", "{line}");
    assert_eq!(
        field(&line, "H2"),
        "hook-env_-s_zsh_--reason_chpwd",
        "{line}"
    );
    assert_eq!(
        hook_calls(&log),
        [
            (pid, "hook-env -s zsh"),
            (pid, "hook-env -s zsh --reason chpwd")
        ],
        "{log}"
    );
}

const HINT: &str = "loading env (mise)...";
const ERASE: &str = "\r\x1b[K";

/// A gated `no_rcs_envlibs` daemon whose own dir has a `mise.toml`, and a dir
/// with nothing for mise to load.
fn hint_daemon() -> (Daemon, tempfile::TempDir) {
    let d = daemon_with("no_rcs_envlibs", GATE, None);
    std::fs::write(d.dir.path().join("mise.toml"), "").unwrap();
    (d, tempfile::TempDir::new().unwrap())
}

#[test]
fn zsh_shows_the_hint_under_the_prompt_and_while_a_command_waits() {
    let (d, _) = hint_daemon();
    let mut pane = Pane::spawn(&d, "/bin/zsh", d.dir.path());
    pane.until(PROMPT_MARK);
    pane.until(HINT);
    pane.type_line("print -r -- OUT_$((1+1)) MISE=${FAKE_MISE-no}");
    // The waiting command's own line, erased once mise answers.
    let waiting = pane.until(HINT);
    assert!(waiting.contains("OUT_$((1+1))"), "{waiting:?}");
    std::fs::write(d.dir.path().join("go"), "").unwrap();
    let after = pane.until_line("OUT_2");
    assert!(after.starts_with(ERASE), "{after:?}");
    assert!(after.contains("OUT_2 MISE=zsh"), "{after:?}");
}

#[test]
fn zsh_clears_the_hint_when_mise_loads_at_the_prompt() {
    let (d, _) = hint_daemon();
    let mut pane = Pane::spawn(&d, "/bin/zsh", d.dir.path());
    pane.until(PROMPT_MARK);
    pane.until(HINT);
    pane.send("print -r -- OUT_$((1+1)) MISE=${FAKE_MISE-no}");
    std::fs::write(d.dir.path().join("go"), "").unwrap();
    let deadline = Instant::now() + WITHIN;
    while !d.dir.path().join("loaded").exists() {
        assert!(Instant::now() < deadline, "mise never loaded at the prompt");
        std::thread::sleep(Duration::from_millis(20));
    }
    pane.send("\r");
    let after = pane.until_line("OUT_2");
    assert!(!after.contains(HINT), "{after:?}");
    assert!(after.contains("OUT_2 MISE=zsh"), "{after:?}");
}

#[test]
fn zsh_ctrl_c_while_waiting_erases_the_hint_and_the_next_command_loads_mise() {
    let (d, _) = hint_daemon();
    let mut pane = Pane::spawn(&d, "/bin/zsh", d.dir.path());
    pane.until(PROMPT_MARK);
    pane.until(HINT);
    pane.type_line("print -r -- NEVER_$((1+1))");
    pane.until(HINT);
    pane.send("\x03");
    let interrupted = pane.until(PROMPT_MARK);
    assert!(interrupted.contains(ERASE), "{interrupted:?}");
    pane.type_line("print -r -- OUT_$((1+1)) MISE=${FAKE_MISE-no}");
    pane.until(HINT);
    std::fs::write(d.dir.path().join("go"), "").unwrap();
    let after = pane.until_line("OUT_2");
    assert!(after.contains("OUT_2 MISE=zsh"), "{after:?}");
    assert!(!after.contains("NEVER_2"), "{after:?}");
}

#[test]
fn zsh_shows_no_hint_for_a_load_faster_than_its_delay() {
    let (d, _) = hint_daemon();
    std::fs::write(d.dir.path().join("go"), "").unwrap();
    // A 5s delay, so a loaded machine's fake mise still beats it.
    std::fs::write(
        d.dir.path().join("config.json"),
        r#"{"shell_startup": "no_rcs_envlibs", "env": {"__tty7_hint_cs": "500"}}"#,
    )
    .unwrap();
    // A command at once waits in preexec; a later one finds mise loaded at the prompt.
    let mut pane = Pane::spawn(&d, "/bin/zsh", d.dir.path());
    let mut seen = pane.until(PROMPT_MARK);
    pane.type_line("print -r -- OUT_$((1+1)) MISE=${FAKE_MISE-no}");
    seen += &pane.until_line("OUT_2");
    assert!(seen.contains("OUT_2 MISE=zsh"), "{seen:?}");
    assert!(!seen.contains(HINT), "{seen:?}");
    let mut idle = Pane::spawn(&d, "/bin/zsh", d.dir.path());
    let mut seen = idle.until(PROMPT_MARK);
    std::thread::sleep(Duration::from_millis(500));
    idle.type_line("print -r -- OUT_$((1+1)) MISE=${FAKE_MISE-no}");
    seen += &idle.until_line("OUT_2");
    assert!(seen.contains("OUT_2 MISE=zsh"), "{seen:?}");
    assert!(!seen.contains(HINT), "{seen:?}");
}

#[test]
fn zsh_shows_no_hint_where_there_is_nothing_to_load() {
    let (d, elsewhere) = hint_daemon();
    let mut pane = Pane::spawn(&d, "/bin/zsh", elsewhere.path());
    let startup = pane.until(PROMPT_MARK);
    pane.type_line("print -r -- OUT_$((1+1)) MISE=${FAKE_MISE-no}");
    std::fs::write(d.dir.path().join("go"), "").unwrap();
    let after = pane.until_line("OUT_2");
    assert!(
        !startup.contains(HINT) && !after.contains(HINT),
        "{after:?}"
    );
    assert!(after.contains("OUT_2 MISE=zsh"), "{after:?}");
}

#[test]
fn bash_shows_the_hint_while_mise_blocks_the_prompt_then_erases_it() {
    let (d, elsewhere) = hint_daemon();
    let mut pane = Pane::spawn(&d, "/bin/bash", d.dir.path());
    pane.until(HINT);
    std::fs::write(d.dir.path().join("go"), "").unwrap();
    let startup = pane.until(PROMPT_MARK);
    assert!(startup.starts_with(ERASE), "{startup:?}");
    pane.type_line("echo OUT_$((1+1)) MISE=${FAKE_MISE-no}");
    assert!(pane.until_line("OUT_2").contains("OUT_2 MISE=bash"));

    let mut quiet = Pane::spawn(&d, "/bin/bash", elsewhere.path());
    assert!(!quiet.until(PROMPT_MARK).contains(HINT));
}
