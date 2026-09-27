//! The real [`Backend`]: this machine's tty7 server, over its local sockets —
//! and through it, every machine it holds an SSH link to.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use tty7_core::client::{ControlClient, PaneClient, PaneSession};
use tty7_core::core::machine::Machine;
use tty7_core::daemon::control::{
    ControlHello, ControlRequest, PaneAgentState, PaneSeed, ReplyOk, RouteInfo, WorkspaceId,
};
use tty7_core::daemon::protocol::{DaemonMsg, WinSize};
use tty7_core::daemon::router::RouteTarget;
use tty7_mobile_proto::{GridSize, TabCreated};

use crate::serve::{Backend, PaneFeed, Remote};

/// The size the gateway claims when it observes a pane. The daemon ignores it
/// for an observer — the pane keeps the size its window gave it — but the
/// message carries one.
const OBSERVE_SIZE: WinSize = WinSize {
    cols: 80,
    rows: 24,
    cell_w: 8,
    cell_h: 16,
};

/// The grid a tab started from the phone gets when the phone did not say. The
/// same the `tty7` CLI starts its shells at.
const NEW_TAB_SIZE: WinSize = WinSize {
    cols: 120,
    rows: 30,
    cell_w: 8,
    cell_h: 16,
};

pub struct Daemon {
    host: String,
    control: Mutex<Option<ControlClient>>,
    /// A control connection per linked machine, routed through the local
    /// server, by link key.
    remote: Mutex<HashMap<String, ControlClient>>,
}

impl Default for Daemon {
    fn default() -> Daemon {
        Daemon {
            host: hostname(),
            control: Mutex::new(None),
            remote: Mutex::new(HashMap::new()),
        }
    }
}

impl Daemon {
    fn hello(&self) -> ControlHello {
        ControlHello::host_rpc(
            format!("tty7-gateway-{}", std::process::id()),
            self.host.clone(),
        )
    }

    fn request(&self, req: ControlRequest) -> io::Result<ReplyOk> {
        let mut slot = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if !slot.as_ref().is_some_and(ControlClient::is_connected) {
            *slot = Some(ControlClient::connect(&self.hello())?);
        }
        let client = slot.as_ref().expect("connected just above");
        let reply = client.request(req);
        if reply.is_err() {
            *slot = None;
        }
        reply
    }

    fn routes(&self) -> io::Result<Vec<RouteInfo>> {
        match self.request(ControlRequest::Routes)? {
            ReplyOk::Routes(routes) => Ok(routes),
            other => Err(unexpected("Routes", &other)),
        }
    }

    /// Where to route for a linked machine — only over a link the desktop
    /// already holds. Routing to a down one would have the server dial it
    /// afresh, guessing at credentials the phone does not have.
    fn target(&self, key: &str) -> io::Result<RouteTarget> {
        let routes = self.routes()?;
        let route = routes
            .iter()
            .find(|r| r.key == key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no machine {key}")))?;
        if !route.connected {
            return Err(link_down(key));
        }
        route.target().map_err(io::Error::other)
    }

    fn request_on(&self, machine: Option<&str>, req: ControlRequest) -> io::Result<ReplyOk> {
        let Some(key) = machine else {
            return self.request(req);
        };
        let mut clients = self.remote.lock().unwrap_or_else(|e| e.into_inner());
        if !clients.get(key).is_some_and(ControlClient::is_connected) {
            let client = ControlClient::routed(self.target(key)?, &self.hello())?;
            clients.insert(key.to_string(), client);
        }
        let reply = clients[key].request(req);
        if reply.is_err() {
            clients.remove(key);
        }
        reply
    }

    fn panes_on(&self, machine: Option<&str>) -> io::Result<PaneClient> {
        match machine {
            None => Ok(PaneClient::local()),
            Some(key) => Ok(PaneClient::routed(self.target(key)?)),
        }
    }

    fn snapshot_on(&self, machine: Option<&str>) -> io::Result<(Machine, Vec<PaneAgentState>)> {
        let tree = match self.request_on(machine, ControlRequest::MachineGet)? {
            ReplyOk::MachineTree(tree) => *tree,
            other => return Err(unexpected("MachineGet", &other)),
        };
        let agents = match self.request_on(machine, ControlRequest::AgentStates)? {
            ReplyOk::AgentStates(agents) => agents,
            other => return Err(unexpected("AgentStates", &other)),
        };
        Ok((tree, agents))
    }
}

fn link_down(key: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotConnected,
        format!("the desktop's link to {key} is down — reconnect it from tty7 on the computer"),
    )
}

impl Backend for Daemon {
    fn hostname(&self) -> String {
        self.host.clone()
    }

    fn snapshot(&self) -> io::Result<(Machine, Vec<PaneAgentState>)> {
        self.snapshot_on(None)
    }

    fn remotes(&self) -> Vec<Remote> {
        let Ok(routes) = self.routes() else {
            return Vec::new();
        };
        // Links that went away take their routed connections with them.
        self.remote
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|key, _| routes.iter().any(|r| &r.key == key && r.connected));
        routes
            .into_iter()
            .map(|route| Remote {
                name: route.host().unwrap_or(&route.key).to_string(),
                snapshot: route.connected.then(|| self.snapshot_on(Some(&route.key))),
                connected: route.connected,
                key: route.key,
            })
            .collect()
    }

    fn observe(&self, machine: Option<&str>, pane_id: u64) -> io::Result<Box<dyn PaneFeed>> {
        let session = self.panes_on(machine)?.observe(pane_id, OBSERVE_SIZE)?;
        Ok(Box::new(Observed(session)))
    }

    fn send_input(&self, machine: Option<&str>, pane_id: u64, bytes: &[u8]) -> io::Result<()> {
        self.panes_on(machine)?.send_input(pane_id, bytes)
    }

    /// The two steps `tty7 tab new` takes: spawn a shell owned by the
    /// workspace, then hang a tab on it — on whichever machine it lives on.
    fn new_tab(
        &self,
        machine: Option<&str>,
        workspace_id: &str,
        cwd: Option<String>,
        size: Option<GridSize>,
    ) -> io::Result<TabCreated> {
        let workspace: WorkspaceId = workspace_id.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("no workspace {workspace_id}"),
            )
        })?;
        let size = size.map_or(NEW_TAB_SIZE, |s| WinSize {
            cols: s.cols.clamp(20, 500),
            rows: s.rows.clamp(5, 300),
            ..NEW_TAB_SIZE
        });
        let owner = workspace.to_string();
        let session = self.panes_on(machine)?.spawn(
            cwd.as_deref().map(PathBuf::from),
            size,
            None,
            Some(owner.clone()),
            Some(owner),
        )?;
        let pane = session.pane_id();
        session.detach()?;
        let seed = PaneSeed {
            pane,
            cwd,
            ssh_spec: None,
            agent: None,
            shell: None,
        };
        let create = ControlRequest::TabCreate {
            workspace,
            at: None,
            pane: seed,
            tab: None,
        };
        match self.request_on(machine, create)? {
            ReplyOk::TabTree(tab) => Ok(TabCreated {
                tab_id: tab.id.to_string(),
                pane_id: pane,
            }),
            other => Err(unexpected("TabCreate", &other)),
        }
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
