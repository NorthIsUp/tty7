//! A phone without a phone: pair with a gateway, print its tree, and measure
//! typing latency on a pane — the spike's two questions, answerable from a
//! desktop shell.
//!
//! ```sh
//! cargo run -p tty7-mobile-client --example probe -- pair '<code>'
//! cargo run -p tty7-mobile-client --example probe -- tree
//! cargo run -p tty7-mobile-client --example probe -- type <pane-id> 'echo hi'
//! ```
//!
//! `newtab <workspace-id> [cwd]` opens a shell in a new tab, as the app does.
//! Keeps its key and host in `$TTY7_PROBE_DIR` (default `./probe-state`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use iroh::SecretKey;
use tty7_mobile_client::{Host, PaneItem, Session};
use tty7_mobile_proto::{ControlEvent, GridSize};

fn dir() -> PathBuf {
    std::env::var_os("TTY7_PROBE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("probe-state"))
}

fn key() -> Result<SecretKey> {
    let path = dir().join("phone.key");
    if let Ok(bytes) = std::fs::read(&path) {
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("bad key"))?;
        return Ok(SecretKey::from_bytes(&bytes));
    }
    std::fs::create_dir_all(dir())?;
    let key = SecretKey::generate();
    std::fs::write(&path, key.to_bytes())?;
    Ok(key)
}

fn host() -> Result<Host> {
    let bytes = std::fs::read(dir().join("host.json")).context("not paired yet")?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let endpoint = tty7_mobile_client::bind(key()?).await?;
    match args.first().map(String::as_str) {
        Some("pair") => {
            let code = args.get(1).context("pair <code>")?;
            let host = tty7_mobile_client::pair(&endpoint, code, "probe").await?;
            std::fs::write(dir().join("host.json"), serde_json::to_vec(&host)?)?;
            println!("paired with {} ({})", host.name, host.id);
        }
        Some("tree") => {
            let t0 = Instant::now();
            let session = Session::connect(&endpoint, &host()?).await?;
            println!("connected in {:?}", t0.elapsed());
            let (_asks, mut events) = session.control().await?.split();
            match events.next().await? {
                Some(ControlEvent::Tree(tree)) => {
                    println!("{}", serde_json::to_string_pretty(&tree)?)
                }
                other => bail!("unexpected {other:?}"),
            }
            println!("link: {:?}", session.link());
        }
        Some("type") => {
            let pane: u64 = args.get(1).context("type <pane> <text>")?.parse()?;
            let text = args.get(2).context("type <pane> <text>")?;
            let session = Session::connect(&endpoint, &host()?).await?;
            let (mut keys, mut screen) = session.pane(pane).await?;
            // Drain the replay: it ends when the pane goes quiet.
            let mut replay = 0usize;
            while let Ok(Ok(Some(item))) =
                tokio::time::timeout(Duration::from_millis(500), screen.next()).await
            {
                if let PaneItem::Output(b) = item {
                    replay += b.len();
                }
            }
            println!("replay: {replay} bytes");
            // Latency: one keystroke at a time, timed to its echo.
            let mut samples = Vec::new();
            for ch in text.chars() {
                let t0 = Instant::now();
                keys.input(ch.to_string().as_bytes()).await?;
                loop {
                    match tokio::time::timeout(Duration::from_secs(5), screen.next()).await {
                        Ok(Ok(Some(PaneItem::Output(_)))) => break,
                        Ok(Ok(Some(PaneItem::Event(_)))) => continue,
                        other => bail!("no echo: {other:?}"),
                    }
                }
                samples.push(t0.elapsed());
            }
            keys.input(b"\r").await?;
            let mut out = Vec::new();
            while let Ok(Ok(Some(item))) =
                tokio::time::timeout(Duration::from_millis(800), screen.next()).await
            {
                if let PaneItem::Output(b) = item {
                    out.extend(b);
                }
            }
            samples.sort();
            println!(
                "echo latency over {} keys: min {:?} median {:?} max {:?}",
                samples.len(),
                samples.first().unwrap_or(&Duration::ZERO),
                samples.get(samples.len() / 2).unwrap_or(&Duration::ZERO),
                samples.last().unwrap_or(&Duration::ZERO),
            );
            println!("after return:\n{}", String::from_utf8_lossy(&out));
            println!("link: {:?}", session.link());
        }
        Some("newtab") => {
            let ws = args.get(1).context("newtab <workspace-id> [cwd]")?;
            let cwd = args.get(2).cloned();
            let session = Session::connect(&endpoint, &host()?).await?;
            let size = GridSize { cols: 56, rows: 40 };
            let created = session.new_tab(ws, cwd, Some(size)).await?;
            println!(
                "opened tab {} with pane {}",
                created.tab_id, created.pane_id
            );
        }
        _ => bail!("usage: probe pair <code> | tree | type <pane> <text> | newtab <ws> [cwd]"),
    }
    endpoint.close().await;
    Ok(())
}
