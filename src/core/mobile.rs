//! The mobile gateway, run by the local daemon while mobile access is on.
//!
//! The daemon outlives the GUI, which is what makes a phone worth having: the
//! panes it reaches are the daemon's, and they are there with every window
//! closed. So the gateway lives here too, switched by `Config::mobile_access`
//! from Settings. It is a client of this same daemon over the local sockets,
//! exactly as the standalone `tty7-gateway serve` is, and shares its state
//! directory — the two cannot run at once (see `State::lock_serve`).
//!
//! Not every daemon can: one started by `tty7 server start` is the lean
//! `tty7-server`, which carries no gateway. So the GUI checks, and when
//! phone access is on and nothing is serving it starts a helper of its own
//! (`tty7-app --mobile-gateway`), detached like the daemon, which serves until
//! the switch goes off. Whichever of the two takes the lock serves; the other
//! stands by.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use tty7_core::core::config::{Config, config_path};
use tty7_gateway::service::{self, Running};
use tty7_gateway::state::State;

/// How often the daemon looks at whether mobile access was switched.
const POLL: Duration = Duration::from_secs(2);
/// How long a gateway that failed to start waits before trying again: the
/// port may free up, or a standalone `tty7-gateway serve` may exit.
const RETRY: Duration = Duration::from_secs(30);

/// Starts the thread that keeps the gateway running exactly while mobile
/// access is on.
pub fn supervise() {
    let spawned = std::thread::Builder::new()
        .name("mobile-supervisor".into())
        .spawn(|| {
            let mut switch = Switch::default();
            let mut running: Option<Running> = None;
            let mut next_try = Instant::now();
            let mut last_error: Option<String> = None;
            loop {
                let wanted = switch.on();
                if !wanted {
                    if let Some(gateway) = running.take() {
                        gateway.stop();
                    }
                    next_try = Instant::now();
                    last_error = None;
                } else if running.is_none() && Instant::now() >= next_try {
                    match State::open_default().and_then(service::start) {
                        Ok(gateway) => {
                            running = Some(gateway);
                            last_error = None;
                        }
                        Err(e) => {
                            let message = format!("{e:#}");
                            if last_error.as_deref() != Some(&message) {
                                log::warn!(
                                    "mobile access is on, but the gateway did not start: {message}"
                                );
                            }
                            last_error = Some(message);
                            next_try = Instant::now() + RETRY;
                        }
                    }
                }
                std::thread::sleep(POLL);
            }
        });
    if let Err(e) = spawned {
        log::error!("could not start the mobile supervisor: {e}");
    }
}

/// How long the GUI gives the daemon's own gateway to come up before it
/// starts the helper instead.
const DAEMON_GRACE: Duration = Duration::from_secs(5);

/// The flag that runs this binary as the gateway helper.
pub const HELPER_FLAG: &str = "--mobile-gateway";

/// From the GUI: if phone access is on and, after the daemon has had its
/// chance, no gateway is serving, starts the helper.
pub fn ensure_served_soon() {
    let _ = std::thread::Builder::new()
        .name("mobile-check".into())
        .spawn(|| {
            std::thread::sleep(DAEMON_GRACE);
            if !Config::load().mobile_access {
                return;
            }
            let Ok(state) = State::open_default() else {
                return;
            };
            if state.serving() {
                return;
            }
            if let Err(e) = spawn_helper() {
                log::warn!("could not start the mobile gateway helper: {e}");
            }
        });
}

fn spawn_helper() -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg(HELPER_FLAG);
    if let Some(dir) = tty7_core::core::config::config_dir_path() {
        cmd.arg("--config-dir").arg(dir);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    tty7_core::daemon::spawn::detach_helper(&mut cmd);
    cmd.spawn().map(drop)
}

/// The helper's whole life: serve while phone access is on, and leave as
/// soon as it is off — or at once, if some other process is serving.
pub fn run_helper() {
    let mut switch = Switch::default();
    loop {
        if !switch.on() {
            return;
        }
        let Ok(state) = State::open_default() else {
            return;
        };
        match service::start(state.clone()) {
            Ok(gateway) => {
                while switch.on() {
                    std::thread::sleep(POLL);
                }
                gateway.stop();
                return;
            }
            // The daemon's own gateway, or a `tty7-gateway serve`: it is
            // served, and this helper has nothing to do.
            Err(_) if state.serving() => return,
            Err(e) => {
                log::warn!("the mobile gateway helper could not start the gateway: {e:#}");
                std::thread::sleep(RETRY);
            }
        }
    }
}

/// `mobile_access` as the config file says, re-read only when the file
/// changes — the daemon polls it every couple of seconds for as long as it
/// lives.
#[derive(Default)]
struct Switch {
    seen: Option<SystemTime>,
    on: bool,
}

impl Switch {
    fn on(&mut self) -> bool {
        let modified = file().and_then(|f| std::fs::metadata(f).ok()?.modified().ok());
        if modified.is_none() || modified != self.seen {
            self.seen = modified;
            self.on = Config::load().mobile_access;
        }
        self.on
    }
}

fn file() -> Option<PathBuf> {
    config_path("config.json")
}
