//! End-to-end tests: several engines on localhost with mock input backends.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use skerry_core::clipboard::{ClipContent, MemoryClipboard};
use skerry_core::config::Paths;
use skerry_core::engine::{self, Backends, EngineEvent, EngineHandle, EngineOptions, FocusView, PairTarget};
use skerry_core::geometry::{Edge, EdgeSet, Rect};
use skerry_core::input::{Capture, CaptureEvent, Emulation};
use skerry_core::keys::{code, Hotkey, HotkeyAction, OsKind};
use skerry_core::proto::Button;
use tokio::sync::mpsc;

#[derive(Default)]
struct CapState {
    edges: EdgeSet,
    released: Vec<Option<(f64, f64)>>,
    grabs: usize,
}

struct MockCapture(Arc<Mutex<CapState>>);

impl Capture for MockCapture {
    fn set_edges(&self, edges: EdgeSet) {
        self.0.lock().unwrap().edges = edges;
    }
    fn set_hotkeys(&self, _: Vec<(Hotkey, HotkeyAction)>) {}
    fn grab(&self) {
        self.0.lock().unwrap().grabs += 1;
    }
    fn release(&self, warp: Option<(f64, f64)>) {
        self.0.lock().unwrap().released.push(warp);
    }
    fn displays(&self) -> Vec<Rect> {
        vec![Rect::new(0, 0, 1920, 1080)]
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Emu {
    Motion(f64, f64),
    Button(Button, bool),
    Scroll(i32, i32),
    Key(u32, bool),
}

struct MockEmu(Arc<Mutex<Vec<Emu>>>);

impl Emulation for MockEmu {
    fn motion_abs(&mut self, x: f64, y: f64) {
        self.0.lock().unwrap().push(Emu::Motion(x, y));
    }
    fn button(&mut self, b: Button, p: bool) {
        self.0.lock().unwrap().push(Emu::Button(b, p));
    }
    fn scroll(&mut self, x: i32, y: i32) {
        self.0.lock().unwrap().push(Emu::Scroll(x, y));
    }
    fn key(&mut self, c: u32, p: bool) {
        self.0.lock().unwrap().push(Emu::Key(c, p));
    }
}

struct Node {
    h: EngineHandle,
    cap: Arc<Mutex<CapState>>,
    emu: Arc<Mutex<Vec<Emu>>>,
    clip: MemoryClipboard,
    tx: mpsc::UnboundedSender<CaptureEvent>,
    id: String,
    port: u16,
    _dir: tempfile::TempDir,
}

impl Node {
    async fn new(os: OsKind) -> Node {
        Node::with(os, |_| {}).await
    }

    async fn with(os: OsKind, tweak: impl FnOnce(&mut EngineOptions)) -> Node {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = EngineOptions::new(Paths::in_dir(dir.path()));
        opts.discovery = false;
        opts.listen = Some("127.0.0.1:0".parse::<SocketAddr>().unwrap());
        tweak(&mut opts);
        let (tx, rx) = mpsc::unbounded_channel();
        let cap = Arc::new(Mutex::new(CapState::default()));
        let emu = Arc::new(Mutex::new(Vec::new()));
        let clip = MemoryClipboard::default();
        let backends = Backends {
            os,
            capture: Box::new(MockCapture(cap.clone())),
            capture_events: rx,
            emulation: Box::new(MockEmu(emu.clone())),
            screen: Box::new(|| vec![Rect::new(0, 0, 1920, 1080)]),
            clipboard: Box::new(clip.clone()),
        };
        let h = engine::start(opts, backends).await.unwrap();
        let snap = h.snapshot();
        Node { id: snap.me.id.clone(), port: snap.me.port, h, cap, emu, clip, tx, _dir: dir }
    }

    fn emu_has(&self, e: &Emu) -> bool {
        self.emu.lock().unwrap().contains(e)
    }

    fn clip_text(&self) -> Option<String> {
        match self.clip.0.lock().unwrap().clone() {
            Some(ClipContent::Text(t)) => Some(t),
            _ => None,
        }
    }

    fn set_clip(&self, t: &str) {
        *self.clip.0.lock().unwrap() = Some(ClipContent::Text(t.into()));
    }

    fn send(&self, ev: CaptureEvent) {
        self.tx.send(ev).unwrap();
    }
}

async fn wait_for(what: &str, f: impl Fn() -> bool) {
    for _ in 0..400 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// Pair `a` (types the code) with `b` (shows it) using `code` or the real one.
async fn pair(a: &Node, b: &Node, wrong: bool) -> (bool, String) {
    let mut events = a.h.subscribe();
    let s = a.h.pair(PairTarget::Address(format!("127.0.0.1:{}", b.port))).await.unwrap();
    wait_for("code shown on b", || b.h.snapshot().pairings.iter().any(|p| p.stage == "show_code")).await;
    wait_for("a asks for code", || a.h.snapshot().pairings.iter().any(|p| p.stage == "enter_code")).await;
    let code = b.h.snapshot().pairings.iter().find_map(|p| p.code.clone()).unwrap();
    let typed = if wrong {
        if code == "000000" {
            "000001".to_string()
        } else {
            "000000".to_string()
        }
    } else {
        code
    };
    a.h.submit_code(s, &typed);
    loop {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await.expect("pairing result").unwrap() {
            EngineEvent::PairingFinished { session, ok, message } if session == s => return (ok, message),
            _ => {}
        }
    }
}

fn online(a: &Node, b: &Node) -> bool {
    a.h.snapshot().peers.iter().any(|p| p.id == b.id && p.paired && p.available)
}

#[tokio::test]
async fn pair_control_type_copy_and_return() {
    let a = Node::new(OsKind::Macos).await;
    let b = Node::new(OsKind::Windows).await;

    let (ok, msg) = pair(&a, &b, false).await;
    assert!(ok, "{msg}");
    wait_for("both online", || online(&a, &b) && online(&b, &a)).await;

    // Put B to the right of A; B learns A is on its left.
    a.h.set_layout(Edge::Right, Some(b.id.clone()));
    wait_for("hint applied", || b.h.snapshot().layout.left.as_deref() == Some(a.id.as_str())).await;
    wait_for("edge enabled", || a.cap.lock().unwrap().edges.contains(Edge::Right)).await;
    wait_for("b knows a's layout", || b.cap.lock().unwrap().edges.contains(Edge::Left)).await;

    // Cross the edge with something on the clipboard.
    a.set_clip("hello from A");
    a.send(CaptureEvent::Begin { edge: Edge::Right, x: 1919.0, y: 540.0 });
    wait_for("cursor enters B on its left edge", || b.emu_has(&Emu::Motion(0.0, 540.0))).await;
    wait_for("focus", || {
        a.h.snapshot().focus == FocusView::Controlling(b.id.clone())
            && b.h.snapshot().focus == FocusView::ControlledBy(a.id.clone())
    })
    .await;
    wait_for("clipboard followed the cursor", || b.clip_text().as_deref() == Some("hello from A")).await;
    // While controlled, B must not switch on its own edges.
    assert!(b.cap.lock().unwrap().edges.is_empty());

    a.send(CaptureEvent::Motion { dx: 10.0, dy: -40.0 });
    wait_for("relative motion", || b.emu_has(&Emu::Motion(10.0, 500.0))).await;

    // Cmd on the Mac becomes Ctrl on Windows.
    a.send(CaptureEvent::Key { code: code::LEFTMETA, pressed: true });
    a.send(CaptureEvent::Key { code: code::C, pressed: true });
    a.send(CaptureEvent::Key { code: code::C, pressed: false });
    wait_for("translated key", || b.emu_has(&Emu::Key(code::LEFTCTRL, true)) && b.emu_has(&Emu::Key(code::C, false)))
        .await;
    a.send(CaptureEvent::Button { button: Button::Left, pressed: true });
    a.send(CaptureEvent::Scroll { x: 0, y: -120 });
    wait_for("button and scroll", || b.emu_has(&Emu::Button(Button::Left, true)) && b.emu_has(&Emu::Scroll(0, -120)))
        .await;

    // Copy something on B, then move back out through B's left edge.
    b.set_clip("copied on B");
    a.send(CaptureEvent::Motion { dx: -50.0, dy: 0.0 });
    wait_for("control returns to A", || a.cap.lock().unwrap().released.contains(&Some((1917.0, 500.0)))).await;
    wait_for("focus local", || a.h.snapshot().focus == FocusView::Local && b.h.snapshot().focus == FocusView::Local)
        .await;
    // Keys and buttons still held on B are released (no stuck Ctrl).
    wait_for("held input released", || {
        b.emu_has(&Emu::Key(code::LEFTCTRL, false)) && b.emu_has(&Emu::Button(Button::Left, false))
    })
    .await;
    wait_for("clipboard came back", || a.clip_text().as_deref() == Some("copied on B")).await;

    // Hotkey jump and hotkey return.
    a.send(CaptureEvent::Hotkey(HotkeyAction::Switch(Edge::Right)));
    wait_for("hotkey grab", || a.cap.lock().unwrap().grabs == 1).await;
    wait_for("entered via hotkey", || a.h.snapshot().focus == FocusView::Controlling(b.id.clone())).await;
    a.send(CaptureEvent::Hotkey(HotkeyAction::ReturnHome));
    wait_for("hotkey home", || a.h.snapshot().focus == FocusView::Local).await;

    // B going away while A controls it gives control back to A.
    a.send(CaptureEvent::Begin { edge: Edge::Right, x: 1919.0, y: 100.0 });
    wait_for("entered again", || a.h.snapshot().focus == FocusView::Controlling(b.id.clone())).await;
    let released_before = a.cap.lock().unwrap().released.len();
    b.h.shutdown().await;
    wait_for("released after peer left", || a.cap.lock().unwrap().released.len() > released_before).await;
    assert_eq!(a.h.snapshot().focus, FocusView::Local);
}

#[tokio::test]
async fn wrong_code_is_rejected() {
    let a = Node::new(OsKind::Linux).await;
    let b = Node::new(OsKind::Linux).await;
    let (ok, _) = pair(&a, &b, true).await;
    assert!(!ok);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(a.h.snapshot().peers.is_empty());
    assert!(b.h.snapshot().peers.is_empty());
}

#[tokio::test]
async fn hop_across_three_computers() {
    let a = Node::new(OsKind::Linux).await;
    let b = Node::new(OsKind::Windows).await;
    let c = Node::new(OsKind::Macos).await;
    assert!(pair(&a, &b, false).await.0);
    assert!(pair(&b, &c, false).await.0);
    assert!(pair(&a, &c, false).await.0);
    wait_for("all online", || online(&a, &b) && online(&a, &c) && online(&b, &c)).await;

    // A | B | C
    a.h.set_layout(Edge::Right, Some(b.id.clone()));
    b.h.set_layout(Edge::Right, Some(c.id.clone()));
    wait_for("layouts", || {
        c.h.snapshot().layout.left.as_deref() == Some(b.id.as_str())
            && b.h.snapshot().layout.left.as_deref() == Some(a.id.as_str())
            && a.cap.lock().unwrap().edges.contains(Edge::Right)
    })
    .await;
    // A must have B's latest screen info (with C on its right).
    tokio::time::sleep(Duration::from_millis(100)).await;

    a.send(CaptureEvent::Begin { edge: Edge::Right, x: 1919.0, y: 270.0 });
    wait_for("on B", || b.emu_has(&Emu::Motion(0.0, 270.0))).await;
    a.send(CaptureEvent::Motion { dx: 1950.0, dy: 0.0 });
    wait_for("hopped to C", || a.h.snapshot().focus == FocusView::Controlling(c.id.clone())).await;
    wait_for("C got entry", || c.emu_has(&Emu::Motion(0.0, 270.0))).await;
    wait_for("B released", || b.h.snapshot().focus == FocusView::Local).await;

    a.send(CaptureEvent::Motion { dx: -5.0, dy: 0.0 });
    wait_for("back on B", || a.h.snapshot().focus == FocusView::Controlling(b.id.clone())).await;
    wait_for("B entered from right", || b.emu_has(&Emu::Motion(1919.0, 270.0))).await;
}

#[tokio::test]
async fn scan_finds_computers_that_discovery_missed() {
    let b = Node::new(OsKind::Windows).await;
    let b_addr: SocketAddr = format!("127.0.0.1:{}", b.port).parse().unwrap();
    // Discovery is off, so only the scan can find b.
    let a = Node::with(OsKind::Macos, |o| o.scan_extra = vec![b_addr]).await;
    assert!(a.h.snapshot().peers.is_empty());

    a.h.rescan();
    wait_for("b listed under nearby", || a.h.snapshot().peers.iter().any(|p| p.id == b.id && !p.paired)).await;
    let peer = a.h.snapshot().peers.into_iter().find(|p| p.id == b.id).unwrap();
    assert_eq!(peer.os, OsKind::Windows);
    assert_eq!(peer.addr.as_deref(), Some(b_addr.to_string().as_str()));
    // The probe hung up again: nothing is pending on b.
    assert!(b.h.snapshot().pairings.is_empty());
    for _ in 0..3 {
        if !a.h.snapshot().scanning {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    assert!(!a.h.snapshot().scanning, "scan should finish");

    // The scanned computer can be paired by picking it from the list.
    let s = a.h.pair(PairTarget::Device(b.id.clone())).await.unwrap();
    wait_for("code shown on b", || b.h.snapshot().pairings.iter().any(|p| p.stage == "show_code")).await;
    a.h.cancel_pairing(s);
}

#[tokio::test]
async fn reinstalled_computer_does_not_flap() {
    // b is paired, then reinstalled: same address, new identity, paired again.
    let a = Node::new(OsKind::Macos).await;
    let old_b = Node::new(OsKind::Windows).await;
    let (ok, msg) = pair(&a, &old_b, false).await;
    assert!(ok, "{msg}");
    wait_for("old b online", || online(&a, &old_b)).await;
    let port = old_b.port;
    old_b.h.shutdown().await;
    drop(old_b);

    let b = Node::with(OsKind::Windows, |o| o.listen = Some(format!("127.0.0.1:{port}").parse().unwrap())).await;
    let (ok, msg) = pair(&a, &b, false).await;
    assert!(ok, "{msg}");
    wait_for("new b online", || online(&a, &b) && online(&b, &a)).await;

    // The stale entry must not keep redialling b and replacing its connection.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut events = b.h.subscribe();
    tokio::time::sleep(Duration::from_secs(4)).await;
    let mut changes = 0;
    while let Ok(ev) = events.try_recv() {
        if matches!(ev, EngineEvent::State(_)) {
            changes += 1;
        }
    }
    assert!(changes <= 1, "b's state kept changing ({changes} times): the connection is flapping");
    assert!(online(&a, &b));
}
