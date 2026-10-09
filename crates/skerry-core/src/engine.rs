//! The Skerry engine.
//!
//! One engine runs on every computer. It owns the network connections, the
//! pairing state, the layout and the "focus": which computer the local mouse
//! and keyboard are currently driving.
//!
//! Control model: the computer whose physical mouse is moving is the
//! *controller*. When its cursor crosses an edge that has a neighbour, its
//! capture backend starts swallowing local input and the engine keeps a
//! virtual cursor on the neighbour's desktop, sending absolute positions,
//! clicks and keys. The neighbour replays them with its emulation backend.
//! When the virtual cursor leaves the neighbour through an edge that leads
//! back (or on to a third computer), control moves accordingly. Any computer
//! can be the controller.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::clipboard::{ClipContent, ClipHash, ClipboardHandle, ClipboardProvider, CLIP_CHUNK, MAX_CLIPBOARD_BYTES};
use crate::config::{Config, HotkeyBinding, Layout, Paths, PeerConfig, DEFAULT_PORT};
use crate::discovery::{Discovered, Discovery, DiscoveryEvent};
use crate::geometry::{Desktop, Edge, EdgeSet, Rect, Step};
use crate::identity::{device_id_for, fingerprint, Identity};
use crate::input::{BackendStatus, Capture, CaptureEvent, Emulation, ScreenSource};
use crate::keys::{translate, HotkeyAction, OsKind};
use crate::net;
use crate::pairing::{self, Role};
use crate::proto::{Button, ClipKind, Hello, Message, ScreenInfo, PROTOCOL_VERSION};
use crate::transport::{self, ConnEvent, ConnHandle, ConnId, Session};

const TICK: Duration = Duration::from_millis(500);
const PING_EVERY: Duration = Duration::from_secs(2);
const PEER_TIMEOUT: Duration = Duration::from_secs(10);
const PAIRING_TIMEOUT: Duration = Duration::from_secs(180);
const UNPAIRED_CONN_TIMEOUT: Duration = Duration::from_secs(60);
const DISPLAY_POLL: Duration = Duration::from_secs(2);
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(30);
const MANUAL_RETRY: Duration = Duration::from_secs(15);
const PAIR_FAILURE_WINDOW: Duration = Duration::from_secs(300);
const PAIR_FAILURE_LIMIT: usize = 5;
const SCAN_CONNECT_TIMEOUT: Duration = Duration::from_millis(800);
/// How long pairing keeps retrying while macOS may be asking about Local
/// Network access, and how often.
const LOCAL_NETWORK_WAIT: Duration = Duration::from_secs(20);
const LOCAL_NETWORK_RETRY: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub struct EngineOptions {
    pub paths: Paths,
    /// Advertise and browse with mDNS.
    pub discovery: bool,
    /// Address to accept connections on. Defaults to `0.0.0.0:<config port>`.
    pub listen: Option<SocketAddr>,
    /// Extra addresses to probe when scanning the network (used by tests).
    pub scan_extra: Vec<SocketAddr>,
}

impl EngineOptions {
    pub fn new(paths: Paths) -> Self {
        EngineOptions { paths, discovery: true, listen: None, scan_extra: Vec::new() }
    }
}

/// The platform pieces the engine drives.
pub struct Backends {
    pub os: OsKind,
    pub capture: Box<dyn Capture>,
    pub capture_events: mpsc::UnboundedReceiver<CaptureEvent>,
    pub emulation: Box<dyn Emulation>,
    pub screen: Box<dyn ScreenSource>,
    pub clipboard: Box<dyn ClipboardProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PairTarget {
    /// A device found by discovery (its id).
    Device(String),
    /// A host name or IP, optionally with `:port`.
    Address(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SettingsUpdate {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub clipboard_sync: Option<bool>,
    pub swap_cmd_ctrl: Option<bool>,
    pub edge_switching: Option<bool>,
    pub block_switch_while_dragging: Option<bool>,
    pub check_updates: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeView {
    pub id: String,
    pub name: String,
    pub os: OsKind,
    pub fingerprint: String,
    pub port: u16,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub enabled: bool,
    pub clipboard_sync: bool,
    pub swap_cmd_ctrl: bool,
    pub edge_switching: bool,
    pub block_switch_while_dragging: bool,
    pub check_updates: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PeerView {
    pub id: String,
    pub name: String,
    pub os: OsKind,
    pub paired: bool,
    pub online: bool,
    pub available: bool,
    pub discovered: bool,
    pub addr: Option<String>,
    pub fingerprint: Option<String>,
    pub edge: Option<Edge>,
    pub speed: f64,
    pub version: Option<String>,
    /// Why the last attempt to connect failed, in plain language.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "peer", rename_all = "snake_case")]
pub enum FocusView {
    Local,
    Controlling(String),
    ControlledBy(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct PairingView {
    pub session: u64,
    pub peer_name: String,
    pub peer_fingerprint: Option<String>,
    /// "connecting", "enter_code", "show_code" or "verifying".
    pub stage: String,
    /// The code to display (only on the computer showing it).
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub me: MeView,
    pub settings: SettingsView,
    pub layout: Layout,
    pub peers: Vec<PeerView>,
    pub focus: FocusView,
    pub capture: BackendStatus,
    pub emulation: BackendStatus,
    pub pairings: Vec<PairingView>,
    pub hotkeys: Vec<HotkeyBinding>,
    pub manual_peers: Vec<String>,
    pub listen_error: Option<String>,
    /// A network scan is running.
    pub scanning: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    State(Box<Snapshot>),
    PairingFinished { session: u64, ok: bool, message: String },
    Notice { message: String },
}

#[derive(Debug)]
enum Command {
    Pair(PairTarget, oneshot::Sender<Result<u64>>),
    SubmitCode(u64, String),
    CancelPairing(u64),
    SetLayout(Edge, Option<String>),
    Forget(String),
    Settings(SettingsUpdate),
    SetSpeed(String, f64),
    AddManualPeer(String),
    RemoveManualPeer(String),
    Rescan,
    Shutdown(oneshot::Sender<()>),
}

/// Cheap, cloneable handle used by the UI / CLI to talk to the engine.
#[derive(Clone)]
pub struct EngineHandle {
    cmd: mpsc::UnboundedSender<Command>,
    events: broadcast::Sender<EngineEvent>,
    snapshot: Arc<RwLock<Snapshot>>,
}

impl EngineHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.read().unwrap().clone()
    }

    /// Start pairing; returns the pairing session id.
    pub async fn pair(&self, target: PairTarget) -> Result<u64> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::Pair(target, tx)).map_err(|_| anyhow!("engine stopped"))?;
        rx.await.map_err(|_| anyhow!("engine stopped"))?
    }

    pub fn submit_code(&self, session: u64, code: &str) {
        let _ = self.cmd.send(Command::SubmitCode(session, code.to_string()));
    }

    pub fn cancel_pairing(&self, session: u64) {
        let _ = self.cmd.send(Command::CancelPairing(session));
    }

    pub fn set_layout(&self, edge: Edge, peer: Option<String>) {
        let _ = self.cmd.send(Command::SetLayout(edge, peer));
    }

    pub fn forget(&self, id: &str) {
        let _ = self.cmd.send(Command::Forget(id.to_string()));
    }

    pub fn update_settings(&self, s: SettingsUpdate) {
        let _ = self.cmd.send(Command::Settings(s));
    }

    pub fn set_speed(&self, id: &str, speed: f64) {
        let _ = self.cmd.send(Command::SetSpeed(id.to_string(), speed));
    }

    pub fn add_manual_peer(&self, addr: &str) {
        let _ = self.cmd.send(Command::AddManualPeer(addr.to_string()));
    }

    pub fn remove_manual_peer(&self, addr: &str) {
        let _ = self.cmd.send(Command::RemoveManualPeer(addr.to_string()));
    }

    /// Look for computers again: re-query mDNS, retry offline peers now, and
    /// sweep the local network for Skerry.
    pub fn rescan(&self) {
        let _ = self.cmd.send(Command::Rescan);
    }

    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd.send(Command::Shutdown(tx)).is_ok() {
            let _ = rx.await;
        }
    }
}

/// Start the engine on the current tokio runtime.
pub async fn start(opts: EngineOptions, backends: Backends) -> Result<EngineHandle> {
    let identity = Identity::load_or_create(&opts.paths.identity)?;
    let cfg = Config::load(&opts.paths.config)?;
    if !opts.paths.config.exists() {
        cfg.save(&opts.paths.config)?;
    }
    let my_id = identity.device_id();

    let listen = opts.listen.unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], cfg.port)));
    let (listener, listen_error) = match TcpListener::bind(listen).await {
        Ok(l) => (Some(l), None),
        Err(e) => (None, Some(format!("Could not listen on {listen}: {e}"))),
    };
    let port = listener.as_ref().and_then(|l| l.local_addr().ok()).map(|a| a.port()).unwrap_or(cfg.port);

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (events, _) = broadcast::channel(64);
    let (conn_tx, conn_rx) = mpsc::unbounded_channel();
    let (internal_tx, internal_rx) = mpsc::unbounded_channel();
    let (disc_tx, disc_rx) = mpsc::unbounded_channel();

    let hello = Arc::new(RwLock::new(Hello {
        protocol: PROTOCOL_VERSION,
        device_id: my_id.clone(),
        name: cfg.name.clone(),
        os: backends.os,
        app_version: crate::APP_VERSION.to_string(),
        port,
    }));

    if let Some(listener) = listener {
        spawn_listener(listener, identity.private, hello.clone(), internal_tx.clone());
    }

    let discovery = if opts.discovery {
        match Discovery::start(&my_id, &cfg.name, backends.os, port, disc_tx) {
            Ok(d) => Some(d),
            Err(e) => {
                tracing::warn!("discovery unavailable: {e:#}");
                None
            }
        }
    } else {
        drop(disc_tx);
        None
    };

    let capture_status = backends.capture.status();
    let emulation_status = backends.emulation.status();
    let emu_status = Arc::new(Mutex::new(emulation_status.clone()));
    backends.capture.set_hotkeys(cfg.parsed_hotkeys());
    backends.capture.set_block_while_dragging(cfg.block_switch_while_dragging);
    backends.capture.set_edges(EdgeSet::empty());
    let local_desktop = Desktop::new(backends.capture.displays());
    let advertised = backends.screen.displays();

    let mut engine = Engine {
        paths: opts.paths,
        cfg,
        identity,
        my_id,
        os: backends.os,
        port,
        hello,
        capture: backends.capture,
        capture_status,
        emulation_status,
        emu: spawn_emulation(backends.emulation, emu_status.clone()),
        emu_status,
        screen: backends.screen,
        clip: ClipboardHandle::spawn(backends.clipboard),
        discovery,
        conns: HashMap::new(),
        online: HashMap::new(),
        screens: HashMap::new(),
        desktops: HashMap::new(),
        discovered: HashMap::new(),
        focus: Focus::Local,
        controlled_by: None,
        pairings: HashMap::new(),
        next_id: 1,
        reconnect: HashMap::new(),
        manual: HashMap::new(),
        local_desktop,
        advertised,
        last_display_poll: Instant::now(),
        edges: EdgeSet::empty(),
        clip_known: HashMap::new(),
        incoming_clip: HashMap::new(),
        pair_failures: Vec::new(),
        events: events.clone(),
        snapshot: Arc::new(RwLock::new(placeholder_snapshot())),
        internal_tx,
        conn_tx,
        listen_error,
        last_errors: HashMap::new(),
        scanning: false,
        scan_extra: opts.scan_extra,
        dirty: true,
    };
    engine.publish();
    let snapshot = engine.snapshot.clone();
    tokio::spawn(engine.run(cmd_rx, backends.capture_events, conn_rx, internal_rx, disc_rx));

    Ok(EngineHandle { cmd: cmd_tx, events, snapshot })
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Intent {
    Incoming,
    Reconnect(String),
    Manual(String),
    Pair(u64),
    /// Found while sweeping the network.
    Probe,
}

enum Internal {
    Established { session: Session, hello: Hello, intent: Intent },
    DialFailed { intent: Intent, error: String },
    ClipRead { to: String, content: Option<(ClipContent, ClipHash)> },
    ClipDecoded { from: String, content: ClipContent, hash: ClipHash },
    ClipWritten { from: String },
    ScanFinished { probed: usize },
}

struct Conn {
    handle: ConnHandle,
    peer_id: String,
    remote_key: [u8; 32],
    hello: Hello,
    initiator: bool,
    hash: Vec<u8>,
    addr: SocketAddr,
    authed: bool,
    opened: Instant,
    last_rx: Instant,
    last_ping: Instant,
}

#[derive(Debug, Clone)]
enum Focus {
    Local,
    Remote {
        peer: String,
        x: f64,
        y: f64,
        /// Edge of the peer's desktop the cursor came in through.
        entered_via: Edge,
        /// Device the cursor came from (this computer's id or another peer).
        came_from: String,
        /// Where to put the local cursor if the peer disappears.
        home: (f64, f64),
    },
}

enum PairStage {
    Connecting,
    AwaitingCode,
    SentSpake(Option<pairing::Spake>),
    InitiatorConfirm(Vec<u8>),
    AwaitingResult,
    ShowCode(String),
    ResponderConfirm(Vec<u8>),
}

struct Pairing {
    role: Role,
    conn: Option<ConnId>,
    peer_name: String,
    stage: PairStage,
    created: Instant,
}

struct Reconnect {
    in_progress: bool,
    next_at: Instant,
    backoff: Duration,
}

struct IncomingClip {
    id: u64,
    kind: ClipKind,
    len: usize,
    buf: Vec<u8>,
}

enum EmuCmd {
    Motion(f64, f64),
    Button(Button, bool),
    Scroll(i32, i32),
    Key(u32, bool),
    ReleaseAll,
    RefreshStatus,
}

/// Emulation runs on its own thread so slow OS calls never stall the engine.
/// It remembers what is held down so everything can be released when the
/// controller leaves or disconnects (no stuck keys).
fn spawn_emulation(mut emu: Box<dyn Emulation>, status: Arc<Mutex<BackendStatus>>) -> std_mpsc::Sender<EmuCmd> {
    let (tx, rx) = std_mpsc::channel::<EmuCmd>();
    std::thread::Builder::new()
        .name("skerry-emulation".into())
        .spawn(move || {
            let os_repeat = emu.os_key_repeat();
            let mut keys: HashSet<u32> = HashSet::new();
            let mut buttons: HashSet<Button> = HashSet::new();
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    EmuCmd::Motion(x, y) => emu.motion_abs(x, y),
                    EmuCmd::Button(b, pressed) => {
                        if pressed {
                            buttons.insert(b);
                        } else if !buttons.remove(&b) {
                            continue;
                        }
                        emu.button(b, pressed);
                    }
                    EmuCmd::Scroll(x, y) => emu.scroll(x, y),
                    EmuCmd::Key(code, pressed) => {
                        if pressed {
                            if !keys.insert(code) && os_repeat {
                                continue;
                            }
                        } else if !keys.remove(&code) {
                            continue;
                        }
                        emu.key(code, pressed);
                    }
                    EmuCmd::ReleaseAll => {
                        for k in keys.drain() {
                            emu.key(k, false);
                        }
                        for b in buttons.drain() {
                            emu.button(b, false);
                        }
                    }
                    EmuCmd::RefreshStatus => *status.lock().unwrap() = emu.status(),
                }
            }
        })
        .expect("spawning emulation thread");
    tx
}

fn spawn_listener(
    listener: TcpListener,
    private: [u8; 32],
    hello: Arc<RwLock<Hello>>,
    tx: mpsc::UnboundedSender<Internal>,
) {
    tokio::spawn(async move {
        loop {
            let (stream, addr) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
            };
            let tx = tx.clone();
            let hello = hello.read().unwrap().clone();
            tokio::spawn(async move {
                match establish(stream, &private, false, hello).await {
                    Ok((session, hello)) => {
                        let _ = tx.send(Internal::Established { session, hello, intent: Intent::Incoming });
                    }
                    Err(e) => tracing::debug!("incoming connection from {addr} failed: {e:#}"),
                }
            });
        }
    });
}

async fn establish(stream: TcpStream, private: &[u8; 32], initiator: bool, hello: Hello) -> Result<(Session, Hello)> {
    let mut session = transport::handshake(stream, private, initiator).await?;
    session.send(&Message::Hello(hello)).await?;
    let msg = tokio::time::timeout(Duration::from_secs(10), session.recv()).await.context("no hello")??;
    match msg {
        Message::Hello(h) if h.protocol == PROTOCOL_VERSION => Ok((session, h)),
        Message::Hello(h) => {
            bail!("peer speaks protocol {} (we speak {PROTOCOL_VERSION}); update Skerry on both computers", h.protocol)
        }
        _ => bail!("peer did not say hello"),
    }
}

/// Resolve "host", "host:port", "ip" or "ip:port".
fn resolve(target: &str, default_port: u16) -> Result<Vec<SocketAddr>> {
    let t = target.trim();
    if let Ok(a) = t.parse::<SocketAddr>() {
        return Ok(vec![a]);
    }
    if let Ok(ip) = t.parse::<std::net::IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, default_port)]);
    }
    let with_port = if t.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) {
        t.to_string()
    } else {
        format!("{t}:{default_port}")
    };
    let mut addrs: Vec<SocketAddr> =
        with_port.to_socket_addrs().with_context(|| format!("cannot resolve {t}"))?.collect();
    addrs.sort_by_key(|a| !a.is_ipv4());
    if addrs.is_empty() {
        bail!("cannot resolve {t}");
    }
    Ok(addrs)
}

fn placeholder_snapshot() -> Snapshot {
    Snapshot {
        me: MeView {
            id: String::new(),
            name: String::new(),
            os: OsKind::current(),
            fingerprint: String::new(),
            port: DEFAULT_PORT,
            version: crate::APP_VERSION.to_string(),
        },
        settings: SettingsView {
            enabled: true,
            clipboard_sync: true,
            swap_cmd_ctrl: true,
            edge_switching: true,
            block_switch_while_dragging: true,
            check_updates: true,
        },
        layout: Layout::default(),
        peers: vec![],
        focus: FocusView::Local,
        capture: BackendStatus::Ok,
        emulation: BackendStatus::Ok,
        pairings: vec![],
        hotkeys: vec![],
        manual_peers: vec![],
        listen_error: None,
        scanning: false,
    }
}

struct Engine {
    paths: Paths,
    cfg: Config,
    identity: Identity,
    my_id: String,
    os: OsKind,
    port: u16,
    hello: Arc<RwLock<Hello>>,

    capture: Box<dyn Capture>,
    capture_status: BackendStatus,
    emulation_status: BackendStatus,
    emu: std_mpsc::Sender<EmuCmd>,
    emu_status: Arc<Mutex<BackendStatus>>,
    screen: Box<dyn ScreenSource>,
    clip: ClipboardHandle,
    discovery: Option<Discovery>,

    conns: HashMap<ConnId, Conn>,
    /// Peer id -> its authenticated connection.
    online: HashMap<String, ConnId>,
    screens: HashMap<String, ScreenInfo>,
    desktops: HashMap<String, Desktop>,
    discovered: HashMap<String, Discovered>,

    focus: Focus,
    controlled_by: Option<String>,

    pairings: HashMap<u64, Pairing>,
    next_id: u64,
    reconnect: HashMap<String, Reconnect>,
    manual: HashMap<String, Reconnect>,

    local_desktop: Desktop,
    advertised: Vec<Rect>,
    last_display_poll: Instant,
    edges: EdgeSet,

    clip_known: HashMap<String, ClipHash>,
    incoming_clip: HashMap<ConnId, IncomingClip>,
    pair_failures: Vec<Instant>,

    events: broadcast::Sender<EngineEvent>,
    snapshot: Arc<RwLock<Snapshot>>,
    internal_tx: mpsc::UnboundedSender<Internal>,
    conn_tx: mpsc::UnboundedSender<ConnEvent>,
    listen_error: Option<String>,
    /// Plain-language reason the last connection attempt to a peer failed.
    last_errors: HashMap<String, String>,
    scanning: bool,
    scan_extra: Vec<SocketAddr>,
    dirty: bool,
}

impl Engine {
    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<Command>,
        mut capture_rx: mpsc::UnboundedReceiver<CaptureEvent>,
        mut conn_rx: mpsc::UnboundedReceiver<ConnEvent>,
        mut internal_rx: mpsc::UnboundedReceiver<Internal>,
        mut disc_rx: mpsc::UnboundedReceiver<DiscoveryEvent>,
    ) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                Some(ev) = capture_rx.recv() => self.on_capture(ev),
                Some(ev) = conn_rx.recv() => match ev {
                    ConnEvent::Message(id, msg) => self.on_message(id, msg),
                    ConnEvent::Closed(id) => self.close_conn(id),
                },
                Some(ev) = internal_rx.recv() => self.on_internal(ev),
                Some(ev) = disc_rx.recv() => self.on_discovery(ev),
                cmd = cmd_rx.recv() => match cmd {
                    Some(Command::Shutdown(reply)) => {
                        self.shutdown();
                        let _ = reply.send(());
                        break;
                    }
                    Some(c) => self.on_command(c),
                    None => {
                        self.shutdown();
                        break;
                    }
                },
                _ = tick.tick() => self.on_tick(),
            }
            if self.dirty {
                self.publish();
            }
        }
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn save(&mut self) {
        if let Err(e) = self.cfg.save(&self.paths.config) {
            tracing::error!("saving config: {e:#}");
            self.notice(format!("Could not save settings: {e}"));
        }
        self.dirty = true;
    }

    fn notice(&self, message: String) {
        tracing::info!("{message}");
        let _ = self.events.send(EngineEvent::Notice { message });
    }

    // ----- connections ---------------------------------------------------

    fn send_to(&self, peer: &str, msg: Message) {
        if let Some(conn) = self.online.get(peer).and_then(|id| self.conns.get(id)) {
            conn.handle.send(msg);
        }
    }

    fn peer_ready(&self, id: &str) -> bool {
        self.online.contains_key(id)
            && self.screens.get(id).is_some_and(|s| s.available)
            && self.desktops.get(id).is_some_and(|d| !d.is_empty())
    }

    /// A computer's name for log messages.
    fn peer_name(&self, id: &str) -> String {
        self.online
            .get(id)
            .and_then(|c| self.conns.get(c))
            .map(|c| c.hello.name.clone())
            .or_else(|| self.cfg.peer(id).map(|p| p.name.clone()))
            .unwrap_or_else(|| id.to_string())
    }

    fn peer_os(&self, id: &str) -> OsKind {
        self.online
            .get(id)
            .and_then(|c| self.conns.get(c))
            .map(|c| c.hello.os)
            .or_else(|| self.cfg.peer(id).map(|p| p.os))
            .unwrap_or(OsKind::Other)
    }

    fn dial(&self, targets: Vec<String>, intent: Intent) {
        let private = self.identity.private;
        let hello = self.hello.read().unwrap().clone();
        let tx = self.internal_tx.clone();
        let os = self.os;
        // macOS may refuse the first connections while it asks the user about
        // Local Network access (Apple TN3179), so when the user is pairing,
        // keep trying for a while instead of failing at once.
        let lan_wait = match intent {
            Intent::Pair(_) if net::local_network_privacy(os) => LOCAL_NETWORK_WAIT,
            _ => Duration::ZERO,
        };
        tokio::spawn(async move {
            let result = async {
                let addrs = tokio::task::spawn_blocking(move || {
                    let mut all = Vec::new();
                    let mut last_err = None;
                    for t in &targets {
                        match resolve(t, DEFAULT_PORT) {
                            Ok(a) => all.extend(a),
                            Err(e) => last_err = Some(e),
                        }
                    }
                    match (all.is_empty(), last_err) {
                        (true, Some(e)) => Err(e),
                        _ => Ok(net::order_candidates(all, &net::local_ipv4())),
                    }
                })
                .await??;
                let give_up = Instant::now() + lan_wait;
                let (stream, _) = loop {
                    match net::connect_any(addrs.clone()).await {
                        Err(e) if Instant::now() < give_up && net::blocked_by_local_network(&format!("{e:#}"), os) => {
                            tokio::time::sleep(LOCAL_NETWORK_RETRY).await;
                        }
                        result => break result?,
                    }
                };
                establish(stream, &private, true, hello).await
            };
            match result.await {
                Ok((session, peer_hello)) => {
                    let _ = tx.send(Internal::Established { session, hello: peer_hello, intent });
                }
                Err(e) => {
                    let _ = tx.send(Internal::DialFailed { intent, error: format!("{e:#}") });
                }
            }
        });
    }

    /// Sweep the local network for other Skerry computers. Every address
    /// that accepts a connection on the Skerry port gets a full handshake, so
    /// the results are real Skerry installs with their names.
    fn start_scan(&mut self) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.dirty = true;
        let skip: HashSet<std::net::IpAddr> =
            self.online.values().filter_map(|c| self.conns.get(c)).map(|c| c.addr.ip()).collect();
        let mut ports = vec![DEFAULT_PORT];
        if self.cfg.port != DEFAULT_PORT {
            ports.push(self.cfg.port);
        }
        let extra = self.scan_extra.clone();
        let private = self.identity.private;
        let hello = self.hello.read().unwrap().clone();
        let tx = self.internal_tx.clone();
        tokio::spawn(async move {
            let hosts = tokio::task::spawn_blocking(|| net::sweep_hosts(&net::local_ipv4())).await.unwrap_or_default();
            let mut targets: Vec<SocketAddr> = hosts
                .into_iter()
                .filter(|ip| !skip.contains(&std::net::IpAddr::V4(*ip)))
                .flat_map(|ip| ports.iter().map(move |p| SocketAddr::new(ip.into(), *p)))
                .collect();
            targets.extend(extra);
            let probed = targets.len();
            let limit = Arc::new(tokio::sync::Semaphore::new(128));
            let mut set = tokio::task::JoinSet::new();
            for addr in targets {
                let (limit, tx, hello) = (limit.clone(), tx.clone(), hello.clone());
                set.spawn(async move {
                    let Ok(_permit) = limit.acquire().await else { return };
                    let Ok(Ok(stream)) = tokio::time::timeout(SCAN_CONNECT_TIMEOUT, TcpStream::connect(addr)).await
                    else {
                        return;
                    };
                    if let Ok((session, peer_hello)) = establish(stream, &private, true, hello).await {
                        let _ = tx.send(Internal::Established { session, hello: peer_hello, intent: Intent::Probe });
                    }
                });
            }
            while set.join_next().await.is_some() {}
            let _ = tx.send(Internal::ScanFinished { probed });
        });
    }

    fn rescan(&mut self) {
        // Forget what discovery reported earlier; live computers answer again.
        self.discovered.retain(|id, _| self.cfg.peer(id).is_some());
        if let Some(d) = &self.discovery {
            if let Err(e) = d.rescan(&self.cfg.name) {
                tracing::warn!("mDNS rescan failed: {e:#}");
            }
        }
        let now = Instant::now();
        for r in self.reconnect.values_mut().chain(self.manual.values_mut()) {
            if !r.in_progress {
                r.next_at = now;
                r.backoff = Duration::from_secs(1);
            }
        }
        self.start_scan();
        self.dirty = true;
    }

    fn on_established(&mut self, session: Session, hello: Hello, intent: Intent) {
        let peer_id = device_id_for(&session.remote_static);
        match &intent {
            Intent::Reconnect(id) => {
                if let Some(r) = self.reconnect.get_mut(id) {
                    r.in_progress = false;
                }
            }
            Intent::Manual(addr) => {
                if let Some(r) = self.manual.get_mut(addr) {
                    r.in_progress = false;
                }
            }
            _ => {}
        }
        if peer_id == self.my_id {
            if let Intent::Pair(s) = intent {
                self.finish_pairing(s, false, "That address is this computer.".into());
            }
            return;
        }
        if hello.device_id != peer_id {
            tracing::warn!("peer claimed id {} but its key says {peer_id}; dropping", hello.device_id);
            return;
        }

        let conn_id = self.next_id();
        let addr = session.peer_addr;
        let remote_key = session.remote_static;
        let initiator = session.initiator;
        let hash = session.handshake_hash.clone();
        let handle = session.spawn(conn_id, self.conn_tx.clone());
        let now = Instant::now();
        self.conns.insert(
            conn_id,
            Conn {
                handle,
                peer_id: peer_id.clone(),
                remote_key,
                hello: hello.clone(),
                initiator,
                hash,
                addr,
                authed: false,
                opened: now,
                last_rx: now,
                last_ping: now,
            },
        );

        let trusted = self.cfg.peer(&peer_id).map(|p| p.public_key == hex::encode(remote_key));
        match (trusted, intent) {
            (Some(true), intent) => {
                self.authenticate(conn_id);
                if let Intent::Pair(s) = intent {
                    self.finish_pairing(s, true, format!("Already paired with {}.", hello.name));
                }
            }
            (Some(false), _) => {
                // Same id, different key: practically impossible unless forged.
                self.notice(format!("{} presented an unexpected key; connection refused.", hello.name));
                self.close_conn(conn_id);
            }
            (None, Intent::Pair(s)) => {
                let Some(p) = self.pairings.get_mut(&s) else {
                    self.close_conn(conn_id);
                    return;
                };
                p.conn = Some(conn_id);
                p.peer_name = hello.name.clone();
                p.stage = PairStage::AwaitingCode;
                self.conns[&conn_id].handle.send(Message::PairRequest);
                self.dirty = true;
            }
            (None, Intent::Incoming) => {
                // Unknown computer: it may now ask to pair.
            }
            (None, Intent::Probe) => {
                // Found by a network scan: list it under Nearby, then hang up.
                let d = Discovered {
                    id: peer_id.clone(),
                    name: hello.name.clone(),
                    os: hello.os,
                    version: hello.app_version.clone(),
                    addrs: vec![SocketAddr::new(addr.ip(), hello.port)],
                };
                if self.discovered.get(&peer_id) != Some(&d) {
                    self.discovered.insert(peer_id, d);
                    self.dirty = true;
                }
                self.close_conn(conn_id);
            }
            (None, _) => self.close_conn(conn_id),
        }
    }

    fn authenticate(&mut self, conn_id: ConnId) {
        let Some(conn) = self.conns.get_mut(&conn_id) else { return };
        conn.authed = true;
        let pid = conn.peer_id.clone();
        self.last_errors.remove(&pid);
        let name = conn.hello.name.clone();
        let os = conn.hello.os;
        let addr = format!("{}", SocketAddr::new(conn.addr.ip(), conn.hello.port));

        if let Some(pc) = self.cfg.peer_mut(&pid) {
            if pc.name != name || pc.os != os || pc.last_addr.as_deref() != Some(addr.as_str()) {
                pc.name = name;
                pc.os = os;
                pc.last_addr = Some(addr);
                self.save();
            }
        }

        if let Some(&old) = self.online.get(&pid) {
            if old != conn_id {
                // Two connections to the same peer (both dialled at once).
                // Both sides keep the one opened by the smaller device id.
                let dialer = |c: &Conn| if c.initiator { self.my_id.clone() } else { c.peer_id.clone() };
                let keep_new = match (self.conns.get(&old), self.conns.get(&conn_id)) {
                    (Some(o), Some(n)) => dialer(n) <= dialer(o),
                    _ => true,
                };
                if keep_new {
                    self.online.insert(pid.clone(), conn_id);
                    self.drop_conn_quietly(old);
                } else {
                    self.drop_conn_quietly(conn_id);
                    return;
                }
            }
        } else {
            self.online.insert(pid.clone(), conn_id);
        }
        self.reconnect.remove(&pid);
        self.conns[&conn_id].handle.send(Message::Screen(self.screen_info()));
        tracing::info!("connected to {} ({pid})", self.conns[&conn_id].hello.name);
        self.update_edges();
        self.dirty = true;
    }

    /// Remove a connection that has been superseded, without treating the
    /// peer as gone.
    fn drop_conn_quietly(&mut self, conn_id: ConnId) {
        self.incoming_clip.remove(&conn_id);
        self.conns.remove(&conn_id);
    }

    fn close_conn(&mut self, conn_id: ConnId) {
        let Some(conn) = self.conns.remove(&conn_id) else { return };
        conn.handle.send(Message::Bye);
        self.incoming_clip.remove(&conn_id);
        let failed: Vec<u64> = self.pairings.iter().filter(|(_, p)| p.conn == Some(conn_id)).map(|(s, _)| *s).collect();
        for s in failed {
            self.finish_pairing(s, false, "The other computer disconnected.".into());
        }
        if self.online.get(&conn.peer_id) == Some(&conn_id) {
            self.online.remove(&conn.peer_id);
            self.peer_offline(&conn.peer_id);
        }
    }

    fn peer_offline(&mut self, pid: &str) {
        tracing::info!("peer {pid} went offline");
        self.screens.remove(pid);
        self.desktops.remove(pid);
        if let Focus::Remote { peer, home, .. } = &self.focus {
            if peer == pid {
                let home = *home;
                self.focus = Focus::Local;
                self.capture.release(Some(home));
            }
        }
        if self.controlled_by.as_deref() == Some(pid) {
            self.controlled_by = None;
            let _ = self.emu.send(EmuCmd::ReleaseAll);
        }
        if self.cfg.peer(pid).is_some() {
            self.reconnect.insert(
                pid.to_string(),
                Reconnect {
                    in_progress: false,
                    next_at: Instant::now() + Duration::from_secs(1),
                    backoff: Duration::from_secs(1),
                },
            );
        }
        self.update_edges();
        self.dirty = true;
    }

    fn screen_info(&self) -> ScreenInfo {
        ScreenInfo {
            displays: self.advertised.clone(),
            neighbors: self.cfg.layout.as_array(),
            available: self.cfg.enabled,
        }
    }

    fn broadcast_screen(&self) {
        let info = self.screen_info();
        for id in self.online.values() {
            if let Some(c) = self.conns.get(id) {
                c.handle.send(Message::Screen(info.clone()));
            }
        }
    }

    fn update_edges(&mut self) {
        let edges: EdgeSet = if self.cfg.enabled && self.cfg.edge_switching && self.controlled_by.is_none() {
            Edge::ALL.into_iter().filter(|e| self.cfg.layout.get(*e).is_some_and(|id| self.peer_ready(id))).collect()
        } else {
            EdgeSet::empty()
        };
        if edges != self.edges {
            self.edges = edges;
            self.capture.set_edges(edges);
        }
    }

    // ----- messages ------------------------------------------------------

    fn on_message(&mut self, conn_id: ConnId, msg: Message) {
        let Some(conn) = self.conns.get_mut(&conn_id) else { return };
        conn.last_rx = Instant::now();
        let authed = conn.authed;
        let pid = conn.peer_id.clone();

        if matches!(
            msg,
            Message::PairRequest | Message::PairSpake { .. } | Message::PairConfirm { .. } | Message::PairResult { .. }
        ) {
            self.on_pairing_message(conn_id, msg);
            return;
        }
        if !authed {
            if !msg.allowed_before_pairing() {
                tracing::warn!("unpaired peer sent {msg:?}; closing");
                self.close_conn(conn_id);
            } else if let Message::Ping(t) = msg {
                self.conns[&conn_id].handle.send(Message::Pong(t));
            } else if matches!(msg, Message::Bye) {
                self.close_conn(conn_id);
            }
            return;
        }
        // Ignore anything from a connection that lost a duplicate race.
        if self.online.get(&pid) != Some(&conn_id) {
            return;
        }

        let controlled = self.controlled_by.as_deref() == Some(pid.as_str());
        match msg {
            Message::Motion { x, y } if controlled => {
                let _ = self.emu.send(EmuCmd::Motion(x, y));
            }
            Message::Button { button, pressed } if controlled => {
                let _ = self.emu.send(EmuCmd::Button(button, pressed));
            }
            Message::Scroll { x, y } if controlled => {
                let _ = self.emu.send(EmuCmd::Scroll(x, y));
            }
            Message::Key { code, pressed } if controlled => {
                let _ = self.emu.send(EmuCmd::Key(code, pressed));
            }
            Message::Motion { .. } | Message::Button { .. } | Message::Scroll { .. } | Message::Key { .. } => {}
            Message::Enter { x, y } => self.on_enter(&pid, x, y),
            Message::Leave => self.on_leave(&pid),
            Message::Screen(info) => {
                self.desktops.insert(pid.clone(), Desktop::new(info.displays.clone()));
                self.screens.insert(pid, info);
                self.update_edges();
                self.dirty = true;
            }
            Message::LayoutHint { edge } => self.on_layout_hint(&pid, edge),
            Message::ClipBegin { id, kind, len } => {
                let len = len as usize;
                if len <= MAX_CLIPBOARD_BYTES && self.cfg.clipboard_sync {
                    self.incoming_clip
                        .insert(conn_id, IncomingClip { id, kind, len, buf: Vec::with_capacity(len.min(1 << 20)) });
                }
            }
            Message::ClipData { id, data } => {
                if let Some(ic) = self.incoming_clip.get_mut(&conn_id) {
                    if ic.id == id {
                        ic.buf.extend_from_slice(&data);
                        if ic.buf.len() > ic.len {
                            self.incoming_clip.remove(&conn_id);
                        }
                    }
                }
            }
            Message::ClipEnd { id } => {
                if let Some(ic) = self.incoming_clip.remove(&conn_id) {
                    if ic.id == id && ic.buf.len() == ic.len {
                        let tx = self.internal_tx.clone();
                        tokio::task::spawn_blocking(move || match ClipContent::decode(ic.kind, &ic.buf) {
                            Ok(content) => {
                                let hash = content.hash();
                                let _ = tx.send(Internal::ClipDecoded { from: pid, content, hash });
                            }
                            Err(e) => tracing::warn!("bad clipboard data: {e:#}"),
                        });
                    }
                }
            }
            Message::Ping(t) => self.conns[&conn_id].handle.send(Message::Pong(t)),
            Message::Pong(_) | Message::Hello(_) => {}
            Message::Bye => self.close_conn(conn_id),
            Message::PairRequest
            | Message::PairSpake { .. }
            | Message::PairConfirm { .. }
            | Message::PairResult { .. } => {
                unreachable!("handled above")
            }
        }
    }

    fn on_layout_hint(&mut self, pid: &str, edge: Option<Edge>) {
        match edge {
            Some(e) => match self.cfg.layout.get(e) {
                None => {
                    self.cfg.layout.place(e, pid);
                }
                Some(x) if x == pid => return,
                Some(_) => {
                    let name = self.cfg.peer(pid).map(|p| p.name.clone()).unwrap_or_default();
                    self.notice(format!("{name} asked to sit on your {} side, but that spot is taken.", e.name()));
                    return;
                }
            },
            None => {
                if self.cfg.layout.remove(pid).is_none() {
                    return;
                }
            }
        }
        self.save();
        self.broadcast_screen();
        self.update_edges();
    }

    // ----- being controlled ---------------------------------------------

    fn on_enter(&mut self, pid: &str, x: f64, y: f64) {
        if !self.cfg.enabled {
            return;
        }
        if let Focus::Remote { peer, home, .. } = self.focus.clone() {
            // Someone took over while we were controlling another computer.
            self.send_to(&peer, Message::Leave);
            self.focus = Focus::Local;
            self.capture.release(Some(home));
        }
        if self.controlled_by.as_deref().is_some_and(|c| c != pid) {
            let _ = self.emu.send(EmuCmd::ReleaseAll);
        }
        if self.controlled_by.as_deref() != Some(pid) {
            tracing::info!("{} is now controlling this computer", self.peer_name(pid));
        }
        self.controlled_by = Some(pid.to_string());
        let _ = self.emu.send(EmuCmd::Motion(x, y));
        self.update_edges();
        self.dirty = true;
    }

    fn on_leave(&mut self, pid: &str) {
        if self.controlled_by.as_deref() != Some(pid) {
            return;
        }
        let _ = self.emu.send(EmuCmd::ReleaseAll);
        self.controlled_by = None;
        tracing::info!("{} stopped controlling this computer", self.peer_name(pid));
        self.update_edges();
        self.dirty = true;
        // Hand our clipboard to the controller if it changed while here.
        self.sync_clipboard_to(pid);
    }

    // ----- controlling --------------------------------------------------

    fn on_capture(&mut self, ev: CaptureEvent) {
        match ev {
            CaptureEvent::Begin { edge, x, y } => {
                let target = self.cfg.layout.get(edge).map(str::to_string).filter(|id| self.peer_ready(id));
                let ok = self.cfg.enabled && self.controlled_by.is_none() && matches!(self.focus, Focus::Local);
                match (ok, target) {
                    (true, Some(id)) => {
                        let frac = self.local_desktop.fraction(edge, x, y);
                        let p = self.local_desktop.clamp(x, y);
                        let home = self.local_desktop.inset(edge, p, 2.0);
                        let me = self.my_id.clone();
                        self.enter(id, edge.opposite(), frac, me, home);
                    }
                    _ => {
                        // Not switching after all: put the cursor back where it was.
                        tracing::debug!("not switching at the {} edge", edge.name());
                        let p = self.local_desktop.clamp(x, y);
                        self.capture.release(Some(self.local_desktop.inset(edge, p, 2.0)));
                    }
                }
            }
            CaptureEvent::Motion { dx, dy } => self.on_motion(dx, dy),
            CaptureEvent::Button { button, pressed } => self.forward(Message::Button { button, pressed }),
            CaptureEvent::Scroll { x, y } => self.forward(Message::Scroll { x, y }),
            CaptureEvent::Key { code, pressed } => {
                if let Focus::Remote { peer, .. } = &self.focus {
                    let code = translate(code, self.os, self.peer_os(peer), self.cfg.swap_cmd_ctrl);
                    self.send_to(peer, Message::Key { code, pressed });
                }
            }
            CaptureEvent::Hotkey(action) => self.on_hotkey(action),
            CaptureEvent::Status(s) => {
                self.capture_status = s;
                self.dirty = true;
            }
        }
    }

    fn forward(&self, msg: Message) {
        if let Focus::Remote { peer, .. } = &self.focus {
            self.send_to(peer, msg);
        }
    }

    fn enter(&mut self, peer: String, via: Edge, frac: f64, came_from: String, home: (f64, f64)) {
        let Some(desk) = self.desktops.get(&peer) else { return };
        let (x, y) = desk.entry_point(via, frac);
        tracing::info!("controlling {}", self.peer_name(&peer));
        self.send_to(&peer, Message::Enter { x, y });
        self.sync_clipboard_to(&peer);
        self.focus = Focus::Remote { peer, x, y, entered_via: via, came_from, home };
        self.dirty = true;
    }

    /// Who is on the other side of `edge` of `peer`'s desktop.
    fn neighbor_of(&self, peer: &str, edge: Edge, entered_via: Edge, came_from: &str) -> Option<String> {
        let explicit = self.screens.get(peer).and_then(|s| s.neighbor(edge)).map(str::to_string);
        // Without an explicit layout on the peer, the way back is the way in.
        explicit.or_else(|| (edge == entered_via).then(|| came_from.to_string()))
    }

    fn on_motion(&mut self, dx: f64, dy: f64) {
        let Focus::Remote { peer, x, y, entered_via, came_from, .. } = &self.focus else { return };
        let Some(desk) = self.desktops.get(peer) else { return };
        let speed = self.cfg.peer(peer).map(|p| p.speed).unwrap_or(1.0);
        let step = desk.step((*x, *y), dx * speed, dy * speed);
        let (peer, entered_via, came_from) = (peer.clone(), *entered_via, came_from.clone());
        let new_pos = match step {
            Step::Inside(nx, ny) => (nx, ny),
            Step::Exit { edge, frac, clamped } => match self.neighbor_of(&peer, edge, entered_via, &came_from) {
                Some(t) if t == self.my_id => {
                    self.return_home(edge.opposite(), frac);
                    return;
                }
                Some(t) if t != peer && self.peer_ready(&t) => {
                    self.hop(t, edge.opposite(), frac);
                    return;
                }
                _ => clamped,
            },
        };
        if let Focus::Remote { x, y, .. } = &mut self.focus {
            if (*x, *y) == new_pos {
                return;
            }
            (*x, *y) = new_pos;
        }
        self.send_to(&peer, Message::Motion { x: new_pos.0, y: new_pos.1 });
    }

    fn return_home(&mut self, local_edge: Edge, frac: f64) {
        let p = self.local_desktop.entry_point(local_edge, frac);
        let p = self.local_desktop.inset(local_edge, p, 2.0);
        self.return_home_at(p);
    }

    fn return_home_at(&mut self, p: (f64, f64)) {
        let Focus::Remote { peer, .. } = std::mem::replace(&mut self.focus, Focus::Local) else { return };
        tracing::info!("back on this computer (from {})", self.peer_name(&peer));
        self.send_to(&peer, Message::Leave);
        self.capture.release(Some(p));
        self.dirty = true;
    }

    fn hop(&mut self, target: String, via: Edge, frac: f64) {
        let Focus::Remote { peer, home, .. } = std::mem::replace(&mut self.focus, Focus::Local) else { return };
        self.send_to(&peer, Message::Leave);
        self.enter(target, via, frac, peer, home);
    }

    fn on_hotkey(&mut self, action: HotkeyAction) {
        match (action, self.focus.clone()) {
            (HotkeyAction::ReturnHome, Focus::Remote { home, .. }) => self.return_home_at(home),
            (HotkeyAction::ReturnHome, Focus::Local) => {}
            (HotkeyAction::Switch(edge), Focus::Local) => {
                if !self.cfg.enabled || self.controlled_by.is_some() {
                    return;
                }
                let Some(t) = self.cfg.layout.get(edge).map(str::to_string).filter(|id| self.peer_ready(id)) else {
                    return;
                };
                self.capture.grab();
                let home = self.local_desktop.center();
                let me = self.my_id.clone();
                self.enter(t, edge.opposite(), 0.5, me, home);
            }
            (HotkeyAction::Switch(edge), Focus::Remote { peer, entered_via, came_from, .. }) => {
                match self.neighbor_of(&peer, edge, entered_via, &came_from) {
                    Some(t) if t == self.my_id => self.return_home(edge.opposite(), 0.5),
                    Some(t) if t != peer && self.peer_ready(&t) => self.hop(t, edge.opposite(), 0.5),
                    _ => {}
                }
            }
        }
    }

    // ----- clipboard -----------------------------------------------------

    fn sync_clipboard_to(&self, peer: &str) {
        if !self.cfg.clipboard_sync {
            return;
        }
        let clip = self.clip.clone();
        let tx = self.internal_tx.clone();
        let to = peer.to_string();
        tokio::spawn(async move {
            let content = clip.read().await;
            let _ = tx.send(Internal::ClipRead { to, content });
        });
    }

    fn send_clipboard(&mut self, to: String, content: ClipContent, hash: ClipHash) {
        if self.clip_known.get(&to) == Some(&hash) {
            return;
        }
        let Some(handle) = self.online.get(&to).and_then(|c| self.conns.get(c)).map(|c| c.handle.clone()) else {
            return;
        };
        self.clip_known.insert(to, hash);
        let id = self.next_id();
        tokio::spawn(async move {
            let encoded = tokio::task::spawn_blocking(move || content.encode()).await;
            let (kind, bytes) = match encoded {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => {
                    tracing::warn!("cannot encode clipboard: {e:#}");
                    return;
                }
                Err(_) => return,
            };
            if bytes.len() > MAX_CLIPBOARD_BYTES {
                tracing::info!("clipboard too large to share ({} bytes)", bytes.len());
                return;
            }
            if !handle.send_bulk(Message::ClipBegin { id, kind, len: bytes.len() as u64 }).await {
                return;
            }
            for chunk in bytes.chunks(CLIP_CHUNK) {
                if !handle.send_bulk(Message::ClipData { id, data: chunk.to_vec() }).await {
                    return;
                }
            }
            handle.send_bulk(Message::ClipEnd { id }).await;
        });
    }

    fn on_internal(&mut self, ev: Internal) {
        match ev {
            Internal::Established { session, hello, intent } => self.on_established(session, hello, intent),
            Internal::DialFailed { intent, error } => match intent {
                Intent::Pair(s) => {
                    tracing::warn!("pairing connection failed: {error}");
                    self.finish_pairing(s, false, net::friendly_error(&error, self.os));
                }
                Intent::Reconnect(id) => {
                    tracing::info!("reconnect to {id} failed: {error}");
                    let friendly = net::friendly_error(&error, self.os);
                    if self.last_errors.get(&id) != Some(&friendly) {
                        self.last_errors.insert(id.clone(), friendly);
                        self.dirty = true;
                    }
                    if let Some(r) = self.reconnect.get_mut(&id) {
                        r.in_progress = false;
                        r.backoff = (r.backoff * 2).min(MAX_RECONNECT_BACKOFF);
                        r.next_at = Instant::now() + r.backoff;
                    }
                }
                Intent::Manual(addr) => {
                    if let Some(r) = self.manual.get_mut(&addr) {
                        r.in_progress = false;
                        r.next_at = Instant::now() + MANUAL_RETRY;
                    }
                }
                Intent::Incoming | Intent::Probe => {}
            },
            Internal::ScanFinished { probed } => {
                tracing::info!("network scan finished ({probed} addresses probed)");
                self.scanning = false;
                self.dirty = true;
            }
            Internal::ClipRead { to, content } => {
                if let Some((content, hash)) = content {
                    self.send_clipboard(to, content, hash);
                }
            }
            Internal::ClipDecoded { from, content, hash } => {
                if !self.cfg.clipboard_sync {
                    return;
                }
                tracing::debug!("clipboard from {from}: {}", content.describe());
                self.clip_known.insert(from.clone(), hash);
                let clip = self.clip.clone();
                let tx = self.internal_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = clip.write(content).await {
                        tracing::warn!("setting clipboard failed: {e:#}");
                    }
                    let _ = tx.send(Internal::ClipWritten { from });
                });
            }
            Internal::ClipWritten { from } => {
                // Passing through: forward to the computer now in focus.
                if let Focus::Remote { peer, .. } = &self.focus {
                    if *peer != from {
                        let peer = peer.clone();
                        self.sync_clipboard_to(&peer);
                    }
                }
            }
        }
    }

    // ----- pairing -------------------------------------------------------

    fn finish_pairing(&mut self, session: u64, ok: bool, message: String) {
        if self.pairings.remove(&session).is_none() {
            return;
        }
        tracing::info!("pairing {session}: {message}");
        let _ = self.events.send(EngineEvent::PairingFinished { session, ok, message });
        self.dirty = true;
    }

    fn fail_pairing(&mut self, session: u64, conn_id: ConnId, message: &str, notify_peer: bool) {
        if notify_peer {
            if let Some(c) = self.conns.get(&conn_id) {
                c.handle.send(Message::PairResult { ok: false, reason: message.to_string() });
            }
        }
        self.finish_pairing(session, false, message.to_string());
        if self.conns.get(&conn_id).is_some_and(|c| !c.authed) {
            self.close_conn(conn_id);
        }
    }

    fn trust(&mut self, conn_id: ConnId) {
        let Some(c) = self.conns.get(&conn_id) else { return };
        let pc = PeerConfig {
            id: c.peer_id.clone(),
            name: c.hello.name.clone(),
            public_key: hex::encode(c.remote_key),
            os: c.hello.os,
            last_addr: Some(SocketAddr::new(c.addr.ip(), c.hello.port).to_string()),
            speed: self.cfg.peer(&c.peer_id).map(|p| p.speed).unwrap_or(1.0),
        };
        self.cfg.peers.retain(|p| p.id != pc.id);
        self.cfg.peers.push(pc);
        self.save();
    }

    fn on_pairing_message(&mut self, conn_id: ConnId, msg: Message) {
        let Some(conn) = self.conns.get(&conn_id) else { return };
        let hash = conn.hash.clone();
        let peer_name = conn.hello.name.clone();
        let session = self.pairings.iter().find(|(_, p)| p.conn == Some(conn_id)).map(|(s, _)| *s);

        match msg {
            Message::PairRequest => {
                let now = Instant::now();
                self.pair_failures.retain(|t| now.duration_since(*t) < PAIR_FAILURE_WINDOW);
                if self.pair_failures.len() >= PAIR_FAILURE_LIMIT {
                    conn.handle.send(Message::PairResult {
                        ok: false,
                        reason: "Too many wrong codes. Wait a few minutes and try again.".into(),
                    });
                    return;
                }
                if let Some(s) = session {
                    self.pairings.remove(&s);
                }
                let s = self.next_id();
                let code = pairing::generate_code();
                self.pairings.insert(
                    s,
                    Pairing {
                        role: Role::Responder,
                        conn: Some(conn_id),
                        peer_name: peer_name.clone(),
                        stage: PairStage::ShowCode(code.clone()),
                        created: now,
                    },
                );
                self.notice(format!("{peer_name} wants to pair. Enter code {code} on {peer_name}."));
                self.dirty = true;
            }
            Message::PairSpake { msg } => {
                let Some(s) = session else { return };
                let p = self.pairings.get_mut(&s).unwrap();
                match std::mem::replace(&mut p.stage, PairStage::Connecting) {
                    PairStage::ShowCode(code) => {
                        let (st, out) = pairing::start(Role::Responder, &code);
                        match st.finish(&msg) {
                            Ok(key) => {
                                let mac = pairing::confirmation(&key, Role::Responder, &hash);
                                p.stage = PairStage::ResponderConfirm(key);
                                let h = &self.conns[&conn_id].handle;
                                h.send(Message::PairSpake { msg: out });
                                h.send(Message::PairConfirm { mac });
                                self.dirty = true;
                            }
                            Err(_) => self.fail_pairing(s, conn_id, "Pairing failed.", true),
                        }
                    }
                    PairStage::SentSpake(Some(st)) => match st.finish(&msg) {
                        Ok(key) => {
                            p.stage = PairStage::InitiatorConfirm(key);
                            self.dirty = true;
                        }
                        Err(_) => self.fail_pairing(s, conn_id, "Pairing failed.", true),
                    },
                    other => {
                        p.stage = other;
                    }
                }
            }
            Message::PairConfirm { mac } => {
                let Some(s) = session else { return };
                let p = self.pairings.get_mut(&s).unwrap();
                match std::mem::replace(&mut p.stage, PairStage::Connecting) {
                    PairStage::InitiatorConfirm(key) => {
                        if pairing::verify_confirmation(&key, Role::Responder, &hash, &mac) {
                            let mine = pairing::confirmation(&key, Role::Initiator, &hash);
                            p.stage = PairStage::AwaitingResult;
                            self.conns[&conn_id].handle.send(Message::PairConfirm { mac: mine });
                        } else {
                            self.fail_pairing(s, conn_id, "The code didn't match. Check it and try again.", true);
                        }
                    }
                    PairStage::ResponderConfirm(key) => {
                        if pairing::verify_confirmation(&key, Role::Initiator, &hash, &mac) {
                            self.trust(conn_id);
                            self.conns[&conn_id].handle.send(Message::PairResult { ok: true, reason: String::new() });
                            self.finish_pairing(s, true, format!("Paired with {peer_name}."));
                            self.authenticate(conn_id);
                        } else {
                            self.pair_failures.push(Instant::now());
                            self.fail_pairing(s, conn_id, "The code entered on the other computer was wrong.", true);
                        }
                    }
                    other => {
                        p.stage = other;
                    }
                }
            }
            Message::PairResult { ok, reason } => {
                let Some(s) = session else { return };
                let p = self.pairings.get_mut(&s).unwrap();
                if ok && matches!(p.stage, PairStage::AwaitingResult) {
                    self.trust(conn_id);
                    self.finish_pairing(s, true, format!("Paired with {peer_name}."));
                    self.authenticate(conn_id);
                } else if !ok {
                    let reason = if reason.is_empty() { "Pairing was cancelled.".to_string() } else { reason };
                    self.fail_pairing(s, conn_id, &reason, false);
                }
            }
            _ => {}
        }
    }

    // ----- commands ------------------------------------------------------

    fn on_command(&mut self, cmd: Command) {
        match cmd {
            Command::Pair(target, reply) => {
                let _ = reply.send(self.start_pairing(target));
            }
            Command::SubmitCode(s, code) => {
                let code = pairing::normalize_code(&code);
                let Some(p) = self.pairings.get_mut(&s) else { return };
                if p.role != Role::Initiator || !matches!(p.stage, PairStage::AwaitingCode) {
                    return;
                }
                if code.len() != 6 {
                    self.notice("The pairing code has 6 digits.".into());
                    return;
                }
                let (st, msg) = pairing::start(Role::Initiator, &code);
                p.stage = PairStage::SentSpake(Some(st));
                if let Some(c) = p.conn.and_then(|c| self.conns.get(&c)) {
                    c.handle.send(Message::PairSpake { msg });
                }
                self.dirty = true;
            }
            Command::CancelPairing(s) => {
                if let Some(conn) = self.pairings.get(&s).and_then(|p| p.conn) {
                    self.fail_pairing(s, conn, "Pairing was cancelled.", true);
                } else {
                    self.finish_pairing(s, false, "Pairing was cancelled.".into());
                }
            }
            Command::SetLayout(edge, peer) => self.set_layout(edge, peer),
            Command::Forget(id) => self.forget(&id),
            Command::Settings(u) => self.apply_settings(u),
            Command::SetSpeed(id, speed) => {
                if let Some(p) = self.cfg.peer_mut(&id) {
                    p.speed = speed.clamp(0.1, 10.0);
                    self.save();
                }
            }
            Command::AddManualPeer(addr) => {
                let addr = addr.trim().to_string();
                if !addr.is_empty() && !self.cfg.manual_peers.contains(&addr) {
                    self.cfg.manual_peers.push(addr);
                    self.save();
                }
            }
            Command::Rescan => self.rescan(),
            Command::RemoveManualPeer(addr) => {
                self.cfg.manual_peers.retain(|a| *a != addr);
                self.manual.remove(&addr);
                self.save();
            }
            Command::Shutdown(_) => unreachable!(),
        }
    }

    fn start_pairing(&mut self, target: PairTarget) -> Result<u64> {
        let (targets, name) = match &target {
            PairTarget::Device(id) => {
                if self.cfg.peer(id).is_some() && self.online.contains_key(id) {
                    bail!("Already paired with this computer.");
                }
                let d = self
                    .discovered
                    .get(id)
                    .ok_or_else(|| anyhow!("That computer is no longer visible on the network."))?;
                (d.addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>(), d.name.clone())
            }
            PairTarget::Address(a) => {
                if a.trim().is_empty() {
                    bail!("Enter an address.");
                }
                (vec![a.trim().to_string()], a.trim().to_string())
            }
        };
        let s = self.next_id();
        self.pairings.insert(
            s,
            Pairing {
                role: Role::Initiator,
                conn: None,
                peer_name: name,
                stage: PairStage::Connecting,
                created: Instant::now(),
            },
        );
        self.dial(targets, Intent::Pair(s));
        self.dirty = true;
        Ok(s)
    }

    fn set_layout(&mut self, edge: Edge, peer: Option<String>) {
        match peer {
            Some(id) => {
                if self.cfg.peer(&id).is_none() {
                    return;
                }
                if let Some(displaced) = self.cfg.layout.place(edge, &id) {
                    self.send_to(&displaced, Message::LayoutHint { edge: None });
                }
                self.send_to(&id, Message::LayoutHint { edge: Some(edge.opposite()) });
            }
            None => {
                if let Some(old) = self.cfg.layout.clear(edge) {
                    self.send_to(&old, Message::LayoutHint { edge: None });
                }
            }
        }
        self.save();
        self.broadcast_screen();
        self.update_edges();
    }

    fn forget(&mut self, id: &str) {
        if self.cfg.layout.remove(id).is_some() {
            self.send_to(id, Message::LayoutHint { edge: None });
        }
        self.cfg.peers.retain(|p| p.id != id);
        self.reconnect.remove(id);
        self.clip_known.remove(id);
        if let Some(c) = self.online.get(id).copied() {
            self.close_conn(c);
        }
        self.reconnect.remove(id);
        self.save();
        self.broadcast_screen();
        self.update_edges();
    }

    fn apply_settings(&mut self, u: SettingsUpdate) {
        if let Some(name) = u.name {
            let name = name.trim().to_string();
            if !name.is_empty() && name != self.cfg.name {
                self.cfg.name = name.clone();
                self.hello.write().unwrap().name = name.clone();
                if let Some(d) = &self.discovery {
                    if let Err(e) = d.advertise(&name) {
                        tracing::warn!("re-advertising: {e:#}");
                    }
                }
            }
        }
        if let Some(v) = u.enabled {
            self.cfg.enabled = v;
            if !v {
                if let Focus::Remote { home, .. } = self.focus {
                    self.return_home_at(home);
                }
                if self.controlled_by.take().is_some() {
                    let _ = self.emu.send(EmuCmd::ReleaseAll);
                }
            }
            self.broadcast_screen();
        }
        if let Some(v) = u.clipboard_sync {
            self.cfg.clipboard_sync = v;
        }
        if let Some(v) = u.swap_cmd_ctrl {
            self.cfg.swap_cmd_ctrl = v;
        }
        if let Some(v) = u.edge_switching {
            self.cfg.edge_switching = v;
        }
        if let Some(v) = u.block_switch_while_dragging {
            self.cfg.block_switch_while_dragging = v;
            self.capture.set_block_while_dragging(v);
        }
        if let Some(v) = u.check_updates {
            self.cfg.check_updates = v;
        }
        self.save();
        self.update_edges();
    }

    // ----- discovery and housekeeping -------------------------------------

    fn on_discovery(&mut self, ev: DiscoveryEvent) {
        match ev {
            DiscoveryEvent::Found(d) => {
                if let Some(r) = self.reconnect.get_mut(&d.id) {
                    if !r.in_progress {
                        r.next_at = Instant::now();
                        r.backoff = Duration::from_secs(1);
                    }
                }
                if self.discovered.get(&d.id) != Some(&d) {
                    self.discovered.insert(d.id.clone(), d);
                    self.dirty = true;
                }
            }
            DiscoveryEvent::Lost(id) => {
                if self.discovered.remove(&id).is_some() {
                    self.dirty = true;
                }
            }
        }
    }

    fn on_tick(&mut self) {
        let now = Instant::now();

        // Keepalive and dead connection detection.
        let mut dead = Vec::new();
        for (id, c) in self.conns.iter_mut() {
            if now.duration_since(c.last_rx) > PEER_TIMEOUT {
                dead.push(*id);
                continue;
            }
            let pairing = self.pairings.values().any(|p| p.conn == Some(*id));
            if !c.authed && !pairing && now.duration_since(c.opened) > UNPAIRED_CONN_TIMEOUT {
                dead.push(*id);
                continue;
            }
            if now.duration_since(c.last_ping) >= PING_EVERY {
                c.last_ping = now;
                c.handle.send(Message::Ping(now.elapsed().as_millis() as u64));
            }
        }
        for id in dead {
            self.close_conn(id);
        }

        // Pairing timeouts.
        let expired: Vec<u64> = self
            .pairings
            .iter()
            .filter(|(_, p)| now.duration_since(p.created) > PAIRING_TIMEOUT)
            .map(|(s, _)| *s)
            .collect();
        for s in expired {
            match self.pairings.get(&s).and_then(|p| p.conn) {
                Some(c) => self.fail_pairing(s, c, "Pairing timed out.", true),
                None => self.finish_pairing(s, false, "Pairing timed out.".into()),
            }
        }

        // Reconnect to paired peers.
        let peers: Vec<PeerConfig> = self.cfg.peers.clone();
        for pc in peers {
            if self.online.contains_key(&pc.id) {
                continue;
            }
            let r = self.reconnect.entry(pc.id.clone()).or_insert(Reconnect {
                in_progress: false,
                next_at: now,
                backoff: Duration::from_secs(1),
            });
            if r.in_progress || now < r.next_at {
                continue;
            }
            let mut targets: Vec<String> = self
                .discovered
                .get(&pc.id)
                .map(|d| d.addrs.iter().map(|a| a.to_string()).collect())
                .unwrap_or_default();
            if let Some(a) = &pc.last_addr {
                if !targets.contains(a) {
                    targets.push(a.clone());
                }
            }
            if targets.is_empty() {
                continue;
            }
            r.in_progress = true;
            self.dial(targets, Intent::Reconnect(pc.id.clone()));
        }

        // Manually configured addresses (for networks without multicast).
        let manual = self.cfg.manual_peers.clone();
        for addr in manual {
            let already = self.online.values().filter_map(|c| self.conns.get(c)).any(|c| {
                let a = SocketAddr::new(c.addr.ip(), c.hello.port).to_string();
                a == addr || c.addr.ip().to_string() == addr
            });
            if already {
                continue;
            }
            let r = self.manual.entry(addr.clone()).or_insert(Reconnect {
                in_progress: false,
                next_at: now,
                backoff: MANUAL_RETRY,
            });
            if r.in_progress || now < r.next_at {
                continue;
            }
            r.in_progress = true;
            r.next_at = now + MANUAL_RETRY;
            self.dial(vec![addr.clone()], Intent::Manual(addr));
        }

        // Display configuration changes.
        if now.duration_since(self.last_display_poll) >= DISPLAY_POLL {
            self.last_display_poll = now;
            let local = Desktop::new(self.capture.displays());
            if local != self.local_desktop {
                self.local_desktop = local;
            }
            let adv = self.screen.displays();
            if adv != self.advertised {
                self.advertised = adv;
                self.broadcast_screen();
            }
            let cs = self.capture.status();
            if cs != self.capture_status {
                self.capture_status = cs;
                self.dirty = true;
            }
            // Permissions can be granted while Skerry runs: refresh the status.
            let _ = self.emu.send(EmuCmd::RefreshStatus);
            let es = self.emu_status.lock().unwrap().clone();
            if es != self.emulation_status {
                self.emulation_status = es;
                self.dirty = true;
            }
        }
    }

    fn shutdown(&mut self) {
        if let Focus::Remote { home, .. } = self.focus {
            self.return_home_at(home);
        }
        if self.controlled_by.take().is_some() {
            let _ = self.emu.send(EmuCmd::ReleaseAll);
        }
        self.capture.set_edges(EdgeSet::empty());
        for c in self.conns.values() {
            c.handle.send(Message::Bye);
        }
        if let Some(d) = &self.discovery {
            d.shutdown();
        }
    }

    // ----- snapshot ------------------------------------------------------

    fn publish(&mut self) {
        self.dirty = false;
        let snap = self.build_snapshot();
        *self.snapshot.write().unwrap() = snap.clone();
        let _ = self.events.send(EngineEvent::State(Box::new(snap)));
    }

    fn build_snapshot(&self) -> Snapshot {
        let mut peers: Vec<PeerView> = self
            .cfg
            .peers
            .iter()
            .map(|p| {
                let conn = self.online.get(&p.id).and_then(|c| self.conns.get(c));
                PeerView {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    os: p.os,
                    paired: true,
                    online: conn.is_some(),
                    available: self.peer_ready(&p.id),
                    discovered: self.discovered.contains_key(&p.id),
                    addr: conn
                        .map(|c| SocketAddr::new(c.addr.ip(), c.hello.port).to_string())
                        .or_else(|| p.last_addr.clone()),
                    fingerprint: crate::identity::parse_public_key(&p.public_key).ok().map(|k| fingerprint(&k)),
                    edge: self.cfg.layout.edge_of(&p.id),
                    speed: p.speed,
                    version: conn.map(|c| c.hello.app_version.clone()),
                    last_error: if conn.is_some() { None } else { self.last_errors.get(&p.id).cloned() },
                }
            })
            .collect();
        for d in self.discovered.values() {
            if self.cfg.peer(&d.id).is_some() {
                continue;
            }
            peers.push(PeerView {
                id: d.id.clone(),
                name: d.name.clone(),
                os: d.os,
                paired: false,
                online: false,
                available: false,
                discovered: true,
                addr: d.addrs.first().map(|a| a.to_string()),
                fingerprint: None,
                edge: None,
                speed: 1.0,
                version: Some(d.version.clone()),
                last_error: None,
            });
        }
        peers.sort_by(|a, b| b.paired.cmp(&a.paired).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));

        let focus = match (&self.focus, &self.controlled_by) {
            (Focus::Remote { peer, .. }, _) => FocusView::Controlling(peer.clone()),
            (Focus::Local, Some(by)) => FocusView::ControlledBy(by.clone()),
            (Focus::Local, None) => FocusView::Local,
        };

        let mut pairings: Vec<PairingView> = self
            .pairings
            .iter()
            .map(|(s, p)| {
                let conn = p.conn.and_then(|c| self.conns.get(&c));
                let (stage, code) = match &p.stage {
                    PairStage::Connecting => ("connecting", None),
                    PairStage::AwaitingCode => ("enter_code", None),
                    PairStage::ShowCode(c) => ("show_code", Some(c.clone())),
                    _ => ("verifying", None),
                };
                PairingView {
                    session: *s,
                    peer_name: p.peer_name.clone(),
                    peer_fingerprint: conn.map(|c| fingerprint(&c.remote_key)),
                    stage: stage.to_string(),
                    code,
                }
            })
            .collect();
        pairings.sort_by_key(|p| p.session);

        Snapshot {
            me: MeView {
                id: self.my_id.clone(),
                name: self.cfg.name.clone(),
                os: self.os,
                fingerprint: self.identity.fingerprint(),
                port: self.port,
                version: crate::APP_VERSION.to_string(),
            },
            settings: SettingsView {
                enabled: self.cfg.enabled,
                clipboard_sync: self.cfg.clipboard_sync,
                swap_cmd_ctrl: self.cfg.swap_cmd_ctrl,
                edge_switching: self.cfg.edge_switching,
                block_switch_while_dragging: self.cfg.block_switch_while_dragging,
                check_updates: self.cfg.check_updates,
            },
            layout: self.cfg.layout.clone(),
            peers,
            focus,
            capture: self.capture_status.clone(),
            emulation: self.emulation_status.clone(),
            pairings,
            hotkeys: self.cfg.hotkeys.clone(),
            manual_peers: self.cfg.manual_peers.clone(),
            listen_error: self.listen_error.clone(),
            scanning: self.scanning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_forms() {
        assert_eq!(resolve("10.0.0.5", 24870).unwrap(), vec!["10.0.0.5:24870".parse().unwrap()]);
        assert_eq!(resolve("10.0.0.5:9", 24870).unwrap(), vec!["10.0.0.5:9".parse().unwrap()]);
        assert!(resolve("localhost", 24870).unwrap().iter().all(|a| a.port() == 24870));
    }
}
