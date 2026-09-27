//! The mobile gateway, run by the local daemon while mobile access is on.
//!
//! The daemon outlives the GUI, which is what makes a phone worth having: the
//! panes it reaches are the daemon's, and they are there with every window
//! closed. So the gateway lives here too, switched by `Config::mobile_access`
//! from Settings. It is a client of this same daemon over the local sockets,
//! exactly as the standalone `tty7-gateway serve` is, and shares its state
//! directory — the two cannot run at once (see `State::lock_serve`).

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
