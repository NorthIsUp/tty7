use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use iroh::Endpoint;
use iroh::endpoint::presets;
use tty7_gateway::daemon::{Daemon, hostname};
use tty7_gateway::serve;
use tty7_gateway::state::{Reachable, State};
use tty7_mobile_proto::{ALPN, PairCode};

#[derive(Parser)]
#[command(
    name = "tty7-gateway",
    version,
    about = "Reach this machine's tty7 panes from your phone"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the gateway. Paired phones can reach this machine while it runs.
    Serve,
    /// Show a one-time code for the tty7 app to scan.
    Pair {
        /// How long the code stays valid, in seconds.
        #[arg(long, default_value_t = 600)]
        ttl: u64,
    },
    /// List paired phones.
    Devices,
    /// Unpair a phone, by name or by the start of its id.
    Revoke { device: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let state = State::open_default()?;
    match cli.command {
        Command::Serve => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(serve_forever(state)),
        Command::Pair { ttl } => pair(&state, ttl),
        Command::Devices => {
            let devices = state.devices()?;
            if devices.is_empty() {
                println!("no phones paired — `tty7-gateway pair` makes a code to scan");
            }
            for d in devices {
                println!("{}  {}", &d.id[..d.id.len().min(12)], d.name);
            }
            Ok(())
        }
        Command::Revoke { device } => {
            let gone = state.revoke(&device)?;
            if gone.is_empty() {
                anyhow::bail!("no paired phone matches '{device}'");
            }
            for d in gone {
                println!("unpaired {}", d.name);
            }
            Ok(())
        }
    }
}

async fn serve_forever(state: State) -> Result<()> {
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(state.secret_key()?)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("starting the iroh endpoint")?;
    println!("tty7-gateway: listening as {}", endpoint.id());

    // Keep the addresses `pair` puts in a code current. The local ones are
    // known at once and are all a phone on the same network needs; the relay
    // only arrives once the endpoint gets online, which on a network that
    // blocks the relays is never — a code must not wait for it.
    tokio::spawn({
        let endpoint = endpoint.clone();
        let state = state.clone();
        async move {
            loop {
                let addr = endpoint.addr();
                let reachable = Reachable {
                    relay: addr.relay_urls().next().map(|u| u.to_string()),
                    addrs: addr.ip_addrs().map(|a| a.to_string()).collect(),
                };
                if state.reachable() != reachable {
                    let _ = state.set_reachable(&reachable);
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    });

    serve::run(endpoint, state, Arc::new(Daemon::default())).await;
    Ok(())
}

fn pair(state: &State, ttl: u64) -> Result<()> {
    let host_id = state.secret_key()?.public().to_string();
    let reachable = state.reachable();
    let code = PairCode {
        host_id,
        host_name: hostname(),
        relay: reachable.relay,
        addrs: reachable.addrs,
        secret: state.open_pairing(ttl)?,
    }
    .encode();

    let qr = qrcode::QrCode::new(code.as_bytes()).context("drawing the pairing code")?;
    let art = qr
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .build();
    println!("{art}");
    println!("Scan with the tty7 app, or paste this code into it:\n\n{code}\n");
    println!(
        "Valid for {} minutes, for one phone. `tty7-gateway serve` must be running.",
        ttl / 60
    );
    Ok(())
}
