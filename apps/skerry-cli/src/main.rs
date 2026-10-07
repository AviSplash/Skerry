//! `skerry-cli`: run Skerry from a terminal.
//!
//! ```text
//! skerry-cli run                 # share this computer's mouse and keyboard
//! skerry-cli id                  # show this computer's id and fingerprint
//! ```
//!
//! While `run` is active, type `help` for commands (pairing, layout, ...).

use anyhow::Result;
use clap::{Parser, Subcommand};
use skerry_core::config::Paths;
use skerry_core::engine::{
    self, EngineEvent, EngineHandle, EngineOptions, FocusView, PairTarget, SettingsUpdate, Snapshot,
};
use skerry_core::geometry::Edge;
use skerry_core::identity::Identity;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Parser)]
#[command(name = "skerry-cli", version, about = "Share one mouse and keyboard between computers")]
struct Cli {
    /// Directory for settings and keys (default: the platform config dir).
    #[arg(long, global = true, env = "SKERRY_CONFIG_DIR")]
    config_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run Skerry in the foreground (default).
    Run {
        /// Listen on this address instead of 0.0.0.0:<configured port>.
        #[arg(long)]
        listen: Option<SocketAddr>,
        /// Do not advertise or browse with mDNS.
        #[arg(long)]
        no_discovery: bool,
        /// Show debug logging.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Print this computer's device id and key fingerprint.
    Id,
    /// Print where settings are stored.
    ConfigPath,
}

fn paths(cli: &Cli) -> Result<Paths> {
    Ok(match &cli.config_dir {
        Some(d) => Paths::in_dir(d),
        None => Paths::default_location()?,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    skerry_platform::init_process();
    let cli = Cli::parse();
    let paths = paths(&cli)?;
    match cli.command.unwrap_or(Cmd::Run { listen: None, no_discovery: false, verbose: false }) {
        Cmd::Id => {
            let id = Identity::load_or_create(&paths.identity)?;
            println!("device id:   {}", id.device_id());
            println!("fingerprint: {}", id.fingerprint());
            Ok(())
        }
        Cmd::ConfigPath => {
            println!("{}", paths.config.display());
            Ok(())
        }
        Cmd::Run { listen, no_discovery, verbose } => run(paths, listen, !no_discovery, verbose).await,
    }
}

async fn run(paths: Paths, listen: Option<SocketAddr>, discovery: bool, verbose: bool) -> Result<()> {
    let filter = if verbose { "skerry_core=debug,skerry_platform=debug,info" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()))
        .init();

    let (backends, description) = skerry_platform::backends().await?;
    let mut opts = EngineOptions::new(paths);
    opts.discovery = discovery;
    opts.listen = listen;
    let engine = engine::start(opts, backends).await?;
    let snap = engine.snapshot();
    println!("Skerry {} on {} ({description})", snap.me.version, snap.me.name);
    println!("device id {}  fingerprint {}  port {}", snap.me.id, snap.me.fingerprint, snap.me.port);
    if let Some(e) = &snap.listen_error {
        println!("warning: {e}");
    }
    println!("Type `help` for commands.");

    let mut events = engine.subscribe();
    let mut last = summary(&snap);
    let mut announced = std::collections::HashSet::new();
    let mut scanning = snap.scanning;
    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(EngineEvent::State(s)) => {
                    let now = summary(&s);
                    if now != last {
                        println!("{now}");
                        last = now;
                    }
                    if s.scanning != scanning {
                        scanning = s.scanning;
                        if scanning {
                            println!(">> Scanning the network…");
                        } else {
                            let nearby = s.peers.iter().filter(|p| !p.paired).count();
                            println!(">> Scan finished: {nearby} unpaired computer(s) nearby. Type `devices` to list them.");
                        }
                    }
                    for p in &s.pairings {
                        // Computers showing a code get a Notice; prompt the one typing it.
                        if p.stage == "enter_code" && announced.insert(p.session) {
                            println!(">> Type: code {} <the 6 digits shown on {}>", p.session, p.peer_name);
                        }
                    }
                }
                Ok(EngineEvent::PairingFinished { ok, message, .. }) => {
                    println!("{} {message}", if ok { "OK:" } else { "Failed:" });
                }
                Ok(EngineEvent::Notice { message }) => println!(">> {message}"),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            },
            line = stdin.next_line() => match line {
                Ok(Some(line)) => {
                    if !command(&engine, line.trim()).await {
                        break;
                    }
                }
                // stdin closed (e.g. running as a service): keep going until Ctrl+C.
                _ => {
                    tokio::signal::ctrl_c().await?;
                    break;
                }
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    engine.shutdown().await;
    Ok(())
}

fn summary(s: &Snapshot) -> String {
    let name = |id: &str| s.peers.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| id.to_string());
    let focus = match &s.focus {
        FocusView::Local => "here".to_string(),
        FocusView::Controlling(p) => format!("controlling {}", name(p)),
        FocusView::ControlledBy(p) => format!("controlled by {}", name(p)),
    };
    let online: Vec<String> = s.peers.iter().filter(|p| p.online).map(|p| p.name.clone()).collect();
    format!("[focus: {focus}] [online: {}]", if online.is_empty() { "none".into() } else { online.join(", ") })
}

async fn command(engine: &EngineHandle, line: &str) -> bool {
    let parts: Vec<&str> = line.split_whitespace().collect();
    match parts.as_slice() {
        [] => {}
        ["help"] | ["?"] => println!(
            "devices                         list paired and nearby computers\n\
             scan                            look for computers on this network again\n\
             pair <device-id | host[:port]>  pair with a computer\n\
             code <session> <digits>         enter the code shown on the other computer\n\
             cancel <session>                cancel a pairing\n\
             layout <left|right|top|bottom> <device-id | none>\n\
             forget <device-id>              unpair a computer\n\
             pause | resume                  stop/start sharing\n\
             quit"
        ),
        ["devices"] | ["ls"] => {
            let s = engine.snapshot();
            if s.peers.is_empty() {
                println!("No computers yet. Start Skerry on another computer on this network.");
            }
            for p in s.peers {
                println!(
                    "{:16}  {:20} {:8} {:8} {:7} {}",
                    p.id,
                    p.name,
                    p.os.label(),
                    if p.paired { "paired" } else { "nearby" },
                    if p.online { "online" } else { "offline" },
                    p.edge.map(|e| format!("on the {}", e.name())).unwrap_or_default()
                );
                if let (false, Some(e)) = (p.online, &p.last_error) {
                    println!("{:16}  last error: {e}", "");
                }
            }
        }
        ["scan"] => engine.rescan(),
        ["pair", target] => {
            let s = engine.snapshot();
            let t = if s.peers.iter().any(|p| p.id == *target) {
                PairTarget::Device(target.to_string())
            } else {
                PairTarget::Address(target.to_string())
            };
            match engine.pair(t).await {
                Ok(session) => println!("Pairing session {session} started."),
                Err(e) => println!("Cannot pair: {e}"),
            }
        }
        ["code", session, digits @ ..] => match session.parse() {
            Ok(s) => engine.submit_code(s, &digits.join("")),
            Err(_) => println!("usage: code <session> <digits>"),
        },
        ["cancel", session] => {
            if let Ok(s) = session.parse() {
                engine.cancel_pairing(s);
            }
        }
        ["layout", edge, who] => match Edge::parse(edge) {
            Some(e) => engine.set_layout(e, (*who != "none").then(|| who.to_string())),
            None => println!("edge must be left, right, top or bottom"),
        },
        ["forget", id] => engine.forget(id),
        ["pause"] => engine.update_settings(SettingsUpdate { enabled: Some(false), ..Default::default() }),
        ["resume"] => engine.update_settings(SettingsUpdate { enabled: Some(true), ..Default::default() }),
        ["quit"] | ["exit"] => return false,
        _ => println!("Unknown command. Type `help`."),
    }
    true
}
