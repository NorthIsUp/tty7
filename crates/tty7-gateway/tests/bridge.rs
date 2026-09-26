//! The gateway end to end, over real iroh connections on loopback: a phone
//! pairs, reads the tree, watches a pane and types into it — against a fake
//! machine, so no daemon or PTY is involved.

use std::io;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::Endpoint;
use iroh::endpoint::presets;
use tty7_core::core::machine::{Machine, Tab, Workspace};
use tty7_core::daemon::control::PaneAgentState;
use tty7_core::daemon::protocol::{DaemonMsg, WinSize};
use tty7_gateway::serve::{self, Backend, PaneFeed};
use tty7_gateway::state::State;
use tty7_mobile_client::{PaneItem, Session};
use tty7_mobile_proto::{ALPN, ControlEvent, PairCode, PaneEvent};

const WAIT: Duration = Duration::from_secs(10);

/// One pane, id 1, whose output is a fixed replay followed by whatever is
/// typed into it — an echoing terminal.
struct FakeMachine {
    machine: Machine,
    typed: Mutex<Option<std_mpsc::Sender<Vec<u8>>>>,
}

impl FakeMachine {
    fn new() -> FakeMachine {
        let panes = serde_json::from_value(serde_json::json!([
            {"id": 1, "title": "zsh", "cwd": "/src/demo"},
        ]))
        .unwrap();
        let mut ws = Workspace {
            name: Some("pale-otter".into()),
            ..Workspace::default()
        };
        ws.tabs.push(Tab::leaf(1));
        FakeMachine {
            machine: Machine {
                workspaces: vec![ws],
                panes,
            },
            typed: Mutex::new(None),
        }
    }
}

impl Backend for FakeMachine {
    fn hostname(&self) -> String {
        "fake-host".into()
    }

    fn snapshot(&self) -> io::Result<(Machine, Vec<PaneAgentState>)> {
        Ok((self.machine.clone(), Vec::new()))
    }

    fn observe(&self, pane_id: u64) -> io::Result<Box<dyn PaneFeed>> {
        if pane_id != 1 {
            return Err(io::Error::other(format!("no such pane {pane_id}")));
        }
        let (tx, rx) = std_mpsc::channel();
        *self.typed.lock().unwrap() = Some(tx);
        Ok(Box::new(FakeFeed {
            replay: vec![
                DaemonMsg::Size(WinSize {
                    cols: 100,
                    rows: 30,
                    cell_w: 8,
                    cell_h: 16,
                }),
                DaemonMsg::Snapshot(b"$ ".to_vec()),
            ],
            typed: rx,
        }))
    }

    fn send_input(&self, _pane_id: u64, bytes: &[u8]) -> io::Result<()> {
        let typed = self.typed.lock().unwrap();
        typed
            .as_ref()
            .ok_or_else(|| io::Error::other("not observed"))?
            .send(bytes.to_vec())
            .map_err(io::Error::other)
    }
}

struct FakeFeed {
    replay: Vec<DaemonMsg>,
    typed: std_mpsc::Receiver<Vec<u8>>,
}

impl PaneFeed for FakeFeed {
    fn recv(&mut self, wait: Duration) -> io::Result<Option<DaemonMsg>> {
        if !self.replay.is_empty() {
            return Ok(Some(self.replay.remove(0)));
        }
        match self.typed.recv_timeout(wait) {
            Ok(bytes) if bytes == b"exit\r" => Ok(Some(DaemonMsg::Exited { code: Some(0) })),
            Ok(bytes) => Ok(Some(DaemonMsg::Output(bytes))),
            Err(std_mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(e) => Err(io::Error::other(e)),
        }
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    state: State,
    gateway: Endpoint,
    phone: Endpoint,
}

impl Rig {
    async fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(dir.path().join("mobile")).unwrap();
        let gateway = Endpoint::builder(presets::Minimal)
            .secret_key(state.secret_key().unwrap())
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap();
        tokio::spawn(serve::run(
            gateway.clone(),
            state.clone(),
            Arc::new(FakeMachine::new()),
        ));
        let phone = Endpoint::builder(presets::Minimal).bind().await.unwrap();
        Rig {
            _dir: dir,
            state,
            gateway,
            phone,
        }
    }

    fn code(&self, secret: String) -> String {
        let addr = self.gateway.addr();
        PairCode {
            host_id: self.gateway.id().to_string(),
            host_name: "fake-host".into(),
            relay: None,
            addrs: addr.ip_addrs().map(|a| a.to_string()).collect(),
            secret,
        }
        .encode()
    }

    async fn paired(&self) -> Session {
        let code = self.code(self.state.open_pairing(60).unwrap());
        let host = tty7_mobile_client::pair(&self.phone, &code, "test phone")
            .await
            .expect("pairing");
        assert_eq!(host.id, self.gateway.id().to_string());
        Session::connect(&self.phone, &host).await.unwrap()
    }
}

async fn within<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, fut).await.expect("timed out")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unpaired_phone_is_turned_away() {
    let rig = Rig::new().await;
    // A code whose secret was never offered: the dial works, the pairing and
    // everything after it does not.
    let code = rig.code("not-the-secret".into());
    let err = within(tty7_mobile_client::pair(&rig.phone, &code, "intruder"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("used up or expired"), "{err}");
    assert!(rig.state.devices().unwrap().is_empty());

    let host = tty7_mobile_client::Host {
        id: rig.gateway.id().to_string(),
        name: "fake-host".into(),
        relay: None,
        addrs: rig
            .gateway
            .addr()
            .ip_addrs()
            .map(|a| a.to_string())
            .collect(),
    };
    let session = Session::connect(&rig.phone, &host).await.unwrap();
    let err = within(session.control()).await.err().expect("denied");
    assert!(err.to_string().contains("not paired"), "{err}");
    let err = within(session.pane(1)).await.err().expect("denied");
    assert!(err.to_string().contains("not paired"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paired_phone_reads_the_tree_and_drives_a_pane() {
    let rig = Rig::new().await;
    let session = within(rig.paired()).await;
    assert_eq!(rig.state.devices().unwrap()[0].name, "test phone");

    let (mut asks, mut tree) = within(session.control()).await.unwrap().split();
    let Some(ControlEvent::Tree(first)) = within(tree.next()).await.unwrap() else {
        panic!("expected a tree first");
    };
    assert_eq!(first.host, "fake-host");
    assert_eq!(first.workspaces[0].name, "pale-otter");
    let pane = &first.workspaces[0].tabs[0].panes[0];
    assert_eq!((pane.id, pane.cwd.as_deref()), (1, Some("/src/demo")));
    // Nothing changed, but asked for: it comes again.
    asks.refresh().await.unwrap();
    assert!(matches!(
        within(tree.next()).await.unwrap(),
        Some(ControlEvent::Tree(t)) if t == first
    ));

    let (mut keys, mut screen) = within(session.pane(1)).await.unwrap();
    assert_eq!(
        within(screen.next()).await.unwrap(),
        Some(PaneItem::Event(PaneEvent::Size {
            cols: 100,
            rows: 30
        }))
    );
    assert_eq!(
        within(screen.next()).await.unwrap(),
        Some(PaneItem::Output(b"$ ".to_vec()))
    );
    keys.input(b"ls\r").await.unwrap();
    assert_eq!(
        within(screen.next()).await.unwrap(),
        Some(PaneItem::Output(b"ls\r".to_vec()))
    );
    keys.input(b"exit\r").await.unwrap();
    assert_eq!(
        within(screen.next()).await.unwrap(),
        Some(PaneItem::Event(PaneEvent::Exited { code: Some(0) }))
    );
    assert_eq!(within(screen.next()).await.unwrap(), None, "stream ends");

    let err = within(session.pane(99)).await.err().expect("no such pane");
    assert!(err.to_string().contains("no such pane 99"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_phone_is_cut_off_at_its_next_stream() {
    let rig = Rig::new().await;
    let session = within(rig.paired()).await;
    within(session.control()).await.unwrap();

    let id = rig.phone.id().to_string();
    assert_eq!(rig.state.revoke(&id[..8]).unwrap().len(), 1);
    let err = within(session.control()).await.err().expect("revoked");
    assert!(err.to_string().contains("not paired"), "{err}");
}
