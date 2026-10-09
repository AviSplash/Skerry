//! Windows backend.
//!
//! Capture uses low-level mouse and keyboard hooks (`WH_MOUSE_LL`,
//! `WH_KEYBOARD_LL`) on a dedicated thread with its own message loop. When
//! the cursor touches an enabled edge, the hooks start swallowing input: the
//! cursor is parked in the middle of the primary display under a tiny
//! invisible window (which hides it), and every mouse move is turned into a
//! delta from that point.
//!
//! Emulation uses `SendInput` with absolute virtual-desktop coordinates and
//! scan codes, so the receiving computer's keyboard layout applies.
//!
//! Edge switching pauses while a game (or any app) hides the cursor or locks
//! it inside a window: games read the mouse as raw input, so a flick that
//! drags the hidden cursor to an edge would otherwise hand the keyboard to
//! another computer while the game still seems to follow the mouse. The
//! switching hotkeys keep working.
//!
//! Limitations of Windows itself: input cannot be injected into the secure
//! desktop (UAC prompts, Ctrl+Alt+Del, the lock screen) or into elevated
//! windows unless Skerry also runs elevated.

mod keymap;

use anyhow::{Context, Result};
use skerry_core::geometry::{Desktop, EdgeSet, Rect};
use skerry_core::input::{Capture, CaptureEvent, CaptureSender, Emulation, ScreenSource};
use skerry_core::keys::{Hotkey, HotkeyAction, HotkeyMatcher, KeyVerdict, Mods};
use skerry_core::proto::Button;
use std::cell::Cell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use windows::core::{w, BOOL};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
    MOUSEEVENTF_XUP, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateCursor, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClipCursor, GetCursorInfo,
    GetCursorPos, GetMessageW, GetSystemMetrics, PostThreadMessageW, RegisterClassW, SetCursorPos,
    SetLayeredWindowAttributes, SetWindowPos, SetWindowsHookExW, ShowWindow, TranslateMessage, CURSORINFO,
    CURSOR_SHOWING, HC_ACTION, HWND_TOPMOST, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED,
    LWA_ALPHA, MSG, MSLLHOOKSTRUCT, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SYSKEYDOWN, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, XBUTTON1, XBUTTON2,
};

type Parts = (Box<dyn Capture>, Box<dyn Emulation>, Box<dyn ScreenSource>, String);

pub fn init_process() {
    // Physical pixels everywhere, so hook coordinates, monitor rectangles and
    // SendInput agree on mixed-DPI setups. Fails harmlessly if already set.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn create(tx: CaptureSender) -> Result<Parts> {
    init_process();
    let capture = WinCapture::new(tx)?;
    Ok((Box::new(capture), Box::new(WinEmulation), Box::new(monitors), "Windows".into()))
}

pub fn monitors() -> Vec<Rect> {
    unsafe extern "system" fn cb(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let out = unsafe { &mut *(data.0 as *mut Vec<Rect>) };
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if unsafe { GetMonitorInfoW(m, &mut info) }.as_bool() {
            let r = info.rcMonitor;
            out.push(Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top));
        }
        BOOL(1)
    }
    let mut out: Vec<Rect> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

const MSG_GRAB: u32 = WM_APP + 1;
const MSG_RELEASE: u32 = WM_APP + 2;
const MSG_RELEASE_WARP: u32 = WM_APP + 3;

const BTN_LEFT: u8 = 1;
const BTN_RIGHT: u8 = 2;
const BTN_MIDDLE: u8 = 4;
const BTN_X1: u8 = 8;
const BTN_X2: u8 = 16;

struct Shared {
    tx: CaptureSender,
    edges: AtomicU8,
    block_drag: AtomicBool,
    grabbed: AtomicBool,
    /// Buttons physically held while not capturing.
    buttons: AtomicU8,
    park: Mutex<(i32, i32)>,
    /// Where the cursor was when capture started.
    origin: Mutex<(i32, i32)>,
    desktop: RwLock<Desktop>,
    hotkeys: Mutex<HotkeyMatcher>,
    /// Keys that went down locally before capture started; their key-up
    /// must still reach Windows or they would stay stuck.
    local_keys: Mutex<HashSet<u32>>,
    thread_id: AtomicU32,
    /// Edge switching is paused because an app hides or confines the cursor.
    app_owns_cursor: AtomicBool,
}

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

thread_local! {
    static HIDER: Cell<isize> = const { Cell::new(0) };
}

pub struct WinCapture {
    shared: Arc<Shared>,
}

impl WinCapture {
    fn new(tx: CaptureSender) -> Result<WinCapture> {
        let shared = Arc::new(Shared {
            tx,
            edges: AtomicU8::new(0),
            block_drag: AtomicBool::new(true),
            grabbed: AtomicBool::new(false),
            buttons: AtomicU8::new(0),
            park: Mutex::new((0, 0)),
            origin: Mutex::new((0, 0)),
            desktop: RwLock::new(Desktop::new(monitors())),
            hotkeys: Mutex::new(HotkeyMatcher::default()),
            local_keys: Mutex::new(HashSet::new()),
            thread_id: AtomicU32::new(0),
            app_owns_cursor: AtomicBool::new(false),
        });
        if SHARED.set(shared.clone()).is_err() {
            anyhow::bail!("the Windows capture backend can only be created once per process");
        }
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        std::thread::Builder::new().name("skerry-win-hooks".into()).spawn(move || hook_thread(ready_tx))?;
        ready_rx.recv().context("hook thread exited")??;
        Ok(WinCapture { shared })
    }

    fn post(&self, msg: u32, w: usize, l: isize) {
        let tid = self.shared.thread_id.load(Ordering::Acquire);
        unsafe {
            let _ = PostThreadMessageW(tid, msg, WPARAM(w), LPARAM(l));
        }
    }
}

impl Capture for WinCapture {
    fn set_edges(&self, edges: EdgeSet) {
        self.shared.edges.store(edges.bits(), Ordering::Release);
    }
    fn set_hotkeys(&self, hotkeys: Vec<(Hotkey, HotkeyAction)>) {
        self.shared.hotkeys.lock().unwrap().set_bindings(hotkeys);
    }
    fn set_block_while_dragging(&self, block: bool) {
        self.shared.block_drag.store(block, Ordering::Relaxed);
    }
    fn grab(&self) {
        self.post(MSG_GRAB, 0, 0);
    }
    fn release(&self, warp: Option<(f64, f64)>) {
        match warp {
            Some((x, y)) => self.post(MSG_RELEASE_WARP, x.round() as i32 as u32 as usize, y.round() as i32 as isize),
            None => self.post(MSG_RELEASE, 0, 0),
        }
    }
    fn displays(&self) -> Vec<Rect> {
        let m = monitors();
        *self.shared.desktop.write().unwrap() = Desktop::new(m.clone());
        m
    }
    fn local_input_only(&self) -> bool {
        // The hooks skip injected events (SendInput) unless capturing.
        true
    }
}

fn hook_thread(ready: std::sync::mpsc::Sender<Result<()>>) {
    let shared = SHARED.get().unwrap().clone();
    shared.thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    let setup = (|| -> Result<()> {
        let hmod: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
        unsafe {
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(hmod), 0).context("installing mouse hook")?;
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(hmod), 0)
                .context("installing keyboard hook")?;
        }
        match create_hider(hmod) {
            Ok(h) => HIDER.with(|c| c.set(h.0 as isize)),
            Err(e) => tracing::warn!("cannot create cursor-hiding window: {e:#}"),
        }
        Ok(())
    })();
    let ok = setup.is_ok();
    let _ = ready.send(setup);
    if !ok {
        return;
    }

    let mut msg = MSG::default();
    loop {
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if r.0 <= 0 {
            break;
        }
        match msg.message {
            MSG_GRAB => start_grab(&shared),
            MSG_RELEASE => stop_grab(&shared, None),
            MSG_RELEASE_WARP => stop_grab(&shared, Some((msg.wParam.0 as u32 as i32, msg.lParam.0 as i32))),
            _ => unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            },
        }
    }
}

unsafe extern "system" fn hider_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// A tiny, almost fully transparent top-most window with a blank cursor.
/// While capturing, the parked cursor sits over it and is therefore invisible.
fn create_hider(hmod: HINSTANCE) -> Result<HWND> {
    unsafe {
        let and_mask = [0xffu8; 32 * 4];
        let xor_mask = [0u8; 32 * 4];
        let blank = CreateCursor(Some(hmod), 0, 0, 32, 32, and_mask.as_ptr().cast(), xor_mask.as_ptr().cast())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(hider_proc),
            hInstance: hmod,
            hCursor: blank,
            lpszClassName: w!("SkerryCursorHider"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            w!("SkerryCursorHider"),
            w!(""),
            WS_POPUP,
            0,
            0,
            64,
            64,
            None,
            None,
            Some(hmod),
            None,
        )?;
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA)?;
        Ok(hwnd)
    }
}

fn hider() -> Option<HWND> {
    let h = HIDER.with(|c| c.get());
    (h != 0).then_some(HWND(h as *mut _))
}

/// Begin swallowing input. Runs on the hook thread.
fn start_grab(s: &Shared) {
    let mut cur = POINT::default();
    if unsafe { GetCursorPos(&mut cur) }.is_ok() {
        *s.origin.lock().unwrap() = (cur.x, cur.y);
    }
    let primary = s.desktop.read().unwrap().displays.first().copied().unwrap_or(Rect::new(0, 0, 800, 600));
    let park = (primary.x + primary.w / 2, primary.y + primary.h / 2);
    *s.park.lock().unwrap() = park;
    unsafe {
        if let Some(h) = hider() {
            let _ =
                SetWindowPos(h, Some(HWND_TOPMOST), park.0 - 32, park.1 - 32, 64, 64, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
        let _ = SetCursorPos(park.0, park.1);
    }
    s.grabbed.store(true, Ordering::Release);
}

fn stop_grab(s: &Shared, warp: Option<(i32, i32)>) {
    s.grabbed.store(false, Ordering::Release);
    unsafe {
        if let Some(h) = hider() {
            let _ = ShowWindow(h, SW_HIDE);
        }
        let (x, y) = warp.unwrap_or_else(|| *s.origin.lock().unwrap());
        let _ = SetCursorPos(x, y);
    }
}

/// Is a key or mouse button physically down right now (as Windows sees it)?
fn is_down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

/// Modifier state as Windows reports it. Unlike our own tracking, this can't
/// go stale when a key-up is never delivered to hooks (Win+L, Ctrl+Alt+Del).
fn os_mods() -> Mods {
    let mut m = Mods::NONE;
    if is_down(0x11) {
        m = m.union(Mods::CTRL);
    }
    if is_down(0x12) {
        m = m.union(Mods::ALT);
    }
    if is_down(0x10) {
        m = m.union(Mods::SHIFT);
    }
    if is_down(0x5b) || is_down(0x5c) {
        m = m.union(Mods::META);
    }
    m
}

/// Does the foreground app own the cursor, the way games do? True when the
/// cursor is hidden or confined to part of the desktop (`ClipCursor`). Such
/// an app reads the mouse as raw input, so the cursor reaching an edge says
/// nothing about where the user wants to be.
fn app_owns_cursor() -> bool {
    unsafe {
        let mut info = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        if GetCursorInfo(&mut info).is_ok() && info.flags.0 & CURSOR_SHOWING.0 == 0 {
            return true;
        }
        let mut clip = RECT::default();
        if GetClipCursor(&mut clip).is_ok() {
            let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
            let (vw, vh) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
            let whole = clip.left <= vx && clip.top <= vy && clip.right >= vx + vw && clip.bottom >= vy + vh;
            if !whole {
                return true;
            }
        }
        false
    }
}

fn send(s: &Shared, ev: CaptureEvent) {
    let _ = s.tx.send(ev);
}

fn hiword(v: u32) -> i16 {
    (v >> 16) as u16 as i16
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        if let Some(s) = SHARED.get() {
            let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            if on_mouse(s, wparam.0 as u32, info) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Returns true to swallow the event.
fn on_mouse(s: &Shared, msg: u32, info: &MSLLHOOKSTRUCT) -> bool {
    let grabbed = s.grabbed.load(Ordering::Acquire);
    let injected = info.flags & LLMHF_INJECTED != 0;
    let xbutton = |d: u32| if hiword(d) as u16 == XBUTTON1 { BTN_X1 } else { BTN_X2 };

    if !grabbed {
        if injected {
            return false;
        }
        match msg {
            WM_LBUTTONDOWN => s.buttons.fetch_or(BTN_LEFT, Ordering::Relaxed),
            WM_RBUTTONDOWN => s.buttons.fetch_or(BTN_RIGHT, Ordering::Relaxed),
            WM_MBUTTONDOWN => s.buttons.fetch_or(BTN_MIDDLE, Ordering::Relaxed),
            WM_XBUTTONDOWN => s.buttons.fetch_or(xbutton(info.mouseData), Ordering::Relaxed),
            WM_LBUTTONUP => s.buttons.fetch_and(!BTN_LEFT, Ordering::Relaxed),
            WM_RBUTTONUP => s.buttons.fetch_and(!BTN_RIGHT, Ordering::Relaxed),
            WM_MBUTTONUP => s.buttons.fetch_and(!BTN_MIDDLE, Ordering::Relaxed),
            WM_XBUTTONUP => s.buttons.fetch_and(!xbutton(info.mouseData), Ordering::Relaxed),
            WM_MOUSEMOVE => return maybe_begin(s, info.pt),
            _ => 0,
        };
        return false;
    }

    // Capturing: a button that went down locally before capture must still
    // be released locally.
    let local_up = |bit: u8| s.buttons.fetch_and(!bit, Ordering::Relaxed) & bit != 0;
    match msg {
        WM_MOUSEMOVE => {
            let park = *s.park.lock().unwrap();
            let (dx, dy) = (info.pt.x - park.0, info.pt.y - park.1);
            if dx != 0 || dy != 0 {
                send(s, CaptureEvent::Motion { dx: dx as f64, dy: dy as f64 });
            }
        }
        WM_LBUTTONDOWN => send(s, CaptureEvent::Button { button: Button::Left, pressed: true }),
        WM_RBUTTONDOWN => send(s, CaptureEvent::Button { button: Button::Right, pressed: true }),
        WM_MBUTTONDOWN => send(s, CaptureEvent::Button { button: Button::Middle, pressed: true }),
        WM_XBUTTONDOWN => {
            let b = if xbutton(info.mouseData) == BTN_X1 { Button::Back } else { Button::Forward };
            send(s, CaptureEvent::Button { button: b, pressed: true });
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
            let (bit, button) = match msg {
                WM_LBUTTONUP => (BTN_LEFT, Button::Left),
                WM_RBUTTONUP => (BTN_RIGHT, Button::Right),
                WM_MBUTTONUP => (BTN_MIDDLE, Button::Middle),
                _ if xbutton(info.mouseData) == BTN_X1 => (BTN_X1, Button::Back),
                _ => (BTN_X2, Button::Forward),
            };
            if local_up(bit) {
                return false;
            }
            send(s, CaptureEvent::Button { button, pressed: false });
        }
        WM_MOUSEWHEEL => send(s, CaptureEvent::Scroll { x: 0, y: hiword(info.mouseData) as i32 }),
        WM_MOUSEHWHEEL => send(s, CaptureEvent::Scroll { x: hiword(info.mouseData) as i32, y: 0 }),
        _ => {}
    }
    true
}

fn maybe_begin(s: &Shared, pt: POINT) -> bool {
    let edges = EdgeSet::from_bits(s.edges.load(Ordering::Acquire));
    if edges.is_empty() {
        return false;
    }
    // Ask Windows which buttons are down rather than trusting our own
    // tracking, which a missed button-up would leave stuck (and with it,
    // switching disabled for good).
    let dragging = is_down(0x01) || is_down(0x02) || is_down(0x04);
    if s.block_drag.load(Ordering::Relaxed) && dragging {
        return false;
    }
    let (x, y) = {
        let desk = s.desktop.read().unwrap();
        let (x, y) = desk.clamp(pt.x as f64, pt.y as f64);
        let Some(edge) = desk.edge_at(x, y) else { return false };
        if !edges.contains(edge) {
            return false;
        }
        drop(desk);
        let owned = app_owns_cursor();
        if s.app_owns_cursor.swap(owned, Ordering::Relaxed) != owned {
            if owned {
                tracing::info!("edge switching paused: an app (likely a game) has hidden or locked the cursor");
            } else {
                tracing::info!("edge switching resumed");
            }
        }
        if owned {
            return false;
        }
        start_grab(s);
        send(s, CaptureEvent::Begin { edge, x, y });
        (x, y)
    };
    tracing::debug!("capture started at {x},{y}");
    true
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        if let Some(s) = SHARED.get() {
            let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let pressed = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if on_key(s, info, pressed) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn on_key(s: &Shared, info: &KBDLLHOOKSTRUCT, pressed: bool) -> bool {
    let grabbed = s.grabbed.load(Ordering::Acquire);
    let injected = info.flags.contains(LLKHF_INJECTED);
    if injected && !grabbed {
        return false;
    }
    let Some(code) = keymap::to_evdev(info.scanCode, info.flags.contains(LLKHF_EXTENDED), info.vkCode) else {
        return grabbed;
    };
    // While capturing, keys never reach Windows, so its key state is frozen:
    // use our own tracking then, and Windows' state otherwise.
    let os = if grabbed { None } else { Some(os_mods()) };
    let verdict =
        if injected { KeyVerdict::Pass } else { s.hotkeys.lock().unwrap().on_key_with_mods(code, pressed, os) };
    match verdict {
        KeyVerdict::Fire(action) => {
            send(s, CaptureEvent::Hotkey(action));
            true
        }
        KeyVerdict::Swallow => true,
        KeyVerdict::Pass if !grabbed => {
            let mut local = s.local_keys.lock().unwrap();
            if pressed {
                local.insert(code);
            } else {
                local.remove(&code);
            }
            false
        }
        KeyVerdict::Pass => {
            if !pressed && s.local_keys.lock().unwrap().remove(&code) {
                return false;
            }
            send(s, CaptureEvent::Key { code, pressed });
            true
        }
    }
}

// ---------------------------------------------------------------------------
// Emulation
// ---------------------------------------------------------------------------

pub struct WinEmulation;

fn mouse_input(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn send_inputs(inputs: &[INPUT]) {
    let n = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if n as usize != inputs.len() {
        tracing::debug!("SendInput injected {n} of {} events (blocked by UIPI or secure desktop?)", inputs.len());
    }
}

impl Emulation for WinEmulation {
    fn motion_abs(&mut self, x: f64, y: f64) {
        let (vx, vy, vw, vh) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2),
                GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2),
            )
        };
        let nx = ((x - vx as f64) * 65535.0 / (vw - 1) as f64).round() as i32;
        let ny = ((y - vy as f64) * 65535.0 / (vh - 1) as f64).round() as i32;
        send_inputs(&[mouse_input(nx, ny, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK)]);
    }

    fn button(&mut self, button: Button, pressed: bool) {
        let (flags, data) = match (button, pressed) {
            (Button::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (Button::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
            (Button::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (Button::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (Button::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (Button::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (Button::Back, true) => (MOUSEEVENTF_XDOWN, XBUTTON1 as u32),
            (Button::Back, false) => (MOUSEEVENTF_XUP, XBUTTON1 as u32),
            (Button::Forward, true) => (MOUSEEVENTF_XDOWN, XBUTTON2 as u32),
            (Button::Forward, false) => (MOUSEEVENTF_XUP, XBUTTON2 as u32),
        };
        send_inputs(&[mouse_input(0, 0, data, flags)]);
    }

    fn scroll(&mut self, x: i32, y: i32) {
        let mut inputs = Vec::new();
        if y != 0 {
            inputs.push(mouse_input(0, 0, y as u32, MOUSEEVENTF_WHEEL));
        }
        if x != 0 {
            inputs.push(mouse_input(0, 0, x as u32, MOUSEEVENTF_HWHEEL));
        }
        send_inputs(&inputs);
    }

    fn key(&mut self, code: u32, pressed: bool) {
        let up = if pressed { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP };
        let ki = match keymap::from_evdev(code) {
            Some(keymap::WinKey::Scan { scan, extended }) => KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan,
                dwFlags: KEYEVENTF_SCANCODE | up | if extended { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
            Some(keymap::WinKey::Vk(vk)) => {
                KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: up, time: 0, dwExtraInfo: 0 }
            }
            None => return,
        };
        send_inputs(&[INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki } }]);
    }
}
