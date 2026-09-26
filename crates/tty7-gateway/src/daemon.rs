//! The real [`Backend`]: this machine's tty7 server, over its local sockets.

use std::io;
use std::sync::Mutex;
use std::time::Duration;

use tty7_core::client::{ControlClient, PaneClient, PaneSession};
use tty7_core::core::machine::Machine;
use tty7_core::daemon::control::{ControlHello, ControlRequest, PaneAgentState, ReplyOk};
use tty7_core::daemon::protocol::{DaemonMsg, WinSize};

use crate::serve::{Backend, PaneFeed};

/// The size the gateway claims when it observes a pane. The daemon ignores it
/// for an observer — the pane keeps the size its window gave it — but the
/// message carries one.
const OBSERVE_SIZE: WinSize = WinSize {
    cols: 80,
    rows: 24,
    cell_w: 8,
    cell_h: 16,
};

pub struct Daemon {
    host: String,
    control: Mutex<Option<ControlClient>>,
}

impl Default for Daemon {
    fn default() -> Daemon {
        Daemon {
            host: hostname(),
            control: Mutex::new(None),
        }
    }
}

impl Daemon {
    fn request(&self, req: ControlRequest) -> io::Result<ReplyOk> {
        let mut slot = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if !slot.as_ref().is_some_and(ControlClient::is_connected) {
            let hello = ControlHello::host_rpc(
                format!("tty7-gateway-{}", std::process::id()),
                self.host.clone(),
            );
            *slot = Some(ControlClient::connect(&hello)?);
        }
        let client = slot.as_ref().expect("connected just above");
        let reply = client.request(req);
        if reply.is_err() {
            *slot = None;
        }
        reply
    }
}

impl Backend for Daemon {
    fn hostname(&self) -> String {
        self.host.clone()
    }

    fn snapshot(&self) -> io::Result<(Machine, Vec<PaneAgentState>)> {
        let machine = match self.request(ControlRequest::MachineGet)? {
            ReplyOk::MachineTree(machine) => *machine,
            other => return Err(unexpected("MachineGet", &other)),
        };
        let agents = match self.request(ControlRequest::AgentStates)? {
            ReplyOk::AgentStates(agents) => agents,
            other => return Err(unexpected("AgentStates", &other)),
        };
        Ok((machine, agents))
    }

    fn observe(&self, pane_id: u64) -> io::Result<Box<dyn PaneFeed>> {
        let session = PaneClient::local().observe(pane_id, OBSERVE_SIZE)?;
        Ok(Box::new(Observed(session)))
    }

    fn send_input(&self, pane_id: u64, bytes: &[u8]) -> io::Result<()> {
        PaneClient::local().send_input(pane_id, bytes)
    }
}

struct Observed(PaneSession);

impl PaneFeed for Observed {
    fn recv(&mut self, wait: Duration) -> io::Result<Option<DaemonMsg>> {
        self.0.set_recv_timeout(Some(wait))?;
        match self.0.recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

fn unexpected(req: &str, reply: &ReplyOk) -> io::Error {
    io::Error::other(format!("unexpected answer to {req}: {reply:?}"))
}

/// This machine's name as the phone lists it.
pub fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for its whole length, and gethostname
        // writes at most that many bytes.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let name = String::from_utf8_lossy(&buf[..end]);
            // `studio.local`, `Mac.lan`: the domain is the network's, not the
            // machine's name.
            let name = name.split('.').next().unwrap_or_default();
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "tty7".to_string())
}
