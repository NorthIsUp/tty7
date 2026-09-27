use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use iroh::endpoint::{BindOpts, presets};
use iroh::{Endpoint, SecretKey};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use tty7_gateway::daemon::{Daemon, hostname};
use tty7_gateway::serve::{self, Backend as _};
use tty7_gateway::state::{Reachable, State};
use tty7_mobile_proto::{ALPN, MDNS_SERVICE, PairCode};

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
        Command::Serve => {
            let _lock = state.lock_serve()?;
            let daemon = Arc::new(Daemon::default());
            // Not fatal: tty7 may well be opened after the gateway, and phones
            // are told the same thing until it is.
            if let Err(e) = daemon.snapshot() {
                eprintln!(
                    "tty7-gateway: {}",
                    serve::server_down(&daemon.hostname(), &e)
                );
            }
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(serve_forever(state, daemon))
        }
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

async fn serve_forever(state: State, daemon: Arc<Daemon>) -> Result<()> {
    let endpoint = bind(state.secret_key()?, state.port()).await?;
    if let Some(port) = endpoint
        .bound_sockets()
        .iter()
        .map(SocketAddr::port)
        .find(|&p| p != 0)
        && state.port() != Some(port)
    {
        state.set_port(port).context("remembering the port")?;
    }
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

    serve::run(endpoint, state, daemon).await;
    Ok(())
}

/// Binds the gateway's endpoint on the port it had last time, so the
/// addresses in phones' pairing codes still reach it, and advertises it on the
/// local network. Each of those is given up, with a note, rather than let it
/// keep the gateway from starting: the port may be taken, and multicast may be
/// off.
async fn bind(key: SecretKey, port: Option<u16>) -> Result<Endpoint> {
    let mut attempts = Vec::new();
    if let Some(port) = port {
        attempts.extend([(port, true), (port, false)]);
    }
    attempts.extend([(0, true), (0, false)]);

    let mut failures = Vec::new();
    for (port, mdns) in attempts {
        let builder = Endpoint::builder(presets::N0)
            .secret_key(key.clone())
            .alpns(vec![ALPN.to_vec()])
            .clear_ip_transports()
            .bind_addr(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))?
            // IPv6 is a bonus, as it is in iroh's own defaults.
            .bind_addr_with_opts(
                SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)),
                BindOpts::default().set_is_required(false),
            )?;
        let builder = match mdns {
            true => builder.address_lookup(MdnsAddressLookup::builder().service_name(MDNS_SERVICE)),
            false => builder,
        };
        match builder.bind().await {
            Ok(endpoint) => {
                for failure in failures {
                    eprintln!("tty7-gateway: {failure} — carrying on without it");
                }
                return Ok(endpoint);
            }
            Err(e) => {
                let what = match (port, mdns) {
                    (0, true) => "local network discovery".to_string(),
                    (0, false) => "the iroh endpoint".to_string(),
                    (port, _) => format!("port {port}"),
                };
                failures.push(format!("{what}: {e}"));
            }
        }
    }
    anyhow::bail!("could not start: {}", failures.join("; "))
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
