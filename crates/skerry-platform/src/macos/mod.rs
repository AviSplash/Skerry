//! macOS backend.
//!
//! Capture uses a Quartz event tap at the HID level. When the cursor pushes
//! against an enabled edge, the cursor is frozen in place
//! (`CGAssociateMouseAndMouseCursorPosition(false)`) and hidden, and the tap
//! swallows every event and reports mouse deltas, buttons, scrolling and
//! keys until released.
//!
//! Emulation posts Quartz events. Modifier state is tracked and attached to
//! every event, and click counts are computed so double-clicks work.
//!
//! Both need the Accessibility permission (System Settings → Privacy &
//! Security → Accessibility). Events Skerry posts carry a marker in their
//! user-data field so the tap can ignore them.

mod keymap;

use anyhow::Result;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CGMouseButton, CallbackResult, EventField, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use skerry_core::geometry::{Desktop, Edge, EdgeSet, Rect};
use skerry_core::input::{BackendStatus, Capture, CaptureEvent, CaptureSender, Emulation, ScreenSource};
use skerry_core::keys::{code as k, Hotkey, HotkeyAction, HotkeyMatcher, KeyVerdict, Mods};
use skerry_core::proto::Button;
use std::collections::HashSet;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

type Parts = (Box<dyn Capture>, Box<dyn Emulation>, Box<dyn ScreenSource>, String);

/// Marks events posted by Skerry ("SKRY").
const MAGIC: i64 = 0x534b_5259;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    // macOS 10.15+: the "Input Monitoring" permission needed to see keys.
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
}

/// Ask for Input Monitoring if not yet granted. Skerry's event tap only
/// strictly needs Accessibility, so this is never treated as required; it
/// covers macOS versions that also filter keystrokes without it.
fn request_input_monitoring() -> bool {
    unsafe { CGPreflightListenEventAccess() || CGRequestListenEventAccess() }
}

/// What to tell the user when macOS withholds input access. Names the lists
/// macOS reports as missing, and covers the confusing case where Skerry
/// already looks switched on: macOS ties the approval to the exact build
/// that was approved, so after an update (of an ad-hoc signed build) the
/// switch stays on but no longer applies.
fn permission_hint() -> String {
    let mut lists = vec!["Accessibility"];
    if !unsafe { CGPreflightListenEventAccess() } {
        lists.push("Input Monitoring");
    }
    let lists = lists.join(" and ");
    if let Some(problem) = location_problem() {
        return format!("{problem} Then switch Skerry on in System Settings → Privacy & Security → {lists}.");
    }
    format!(
        "Switch Skerry on in System Settings → Privacy & Security → {lists}. If it's already switched on there, \
         macOS is remembering an older copy of Skerry: click Reset permissions, then allow Skerry again \
         when macOS asks."
    )
}

/// The Skerry.app this program runs from, if it runs from an app bundle.
fn app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // …/Skerry.app/Contents/MacOS/skerry
    let bundle = exe.ancestors().nth(3)?;
    bundle.extension().is_some_and(|e| e == "app").then(|| bundle.to_path_buf())
}

/// Why macOS can't keep Skerry's permissions because of where it runs
/// from: a disk image, or a temporary copy macOS makes of a downloaded app
/// that wasn't moved to Applications (App Translocation, a new path on every
/// launch). `None` when the location is fine.
fn location_problem() -> Option<&'static str> {
    let bundle = app_bundle()?;
    let path = bundle.to_string_lossy();
    if path.contains("/AppTranslocation/") {
        Some(
            "macOS is running Skerry from a temporary copy, so it can't keep Skerry's permissions. Quit Skerry, \
             drag it into Applications, and open it from there.",
        )
    } else if path.starts_with("/Volumes/") {
        Some(
            "Skerry is running from the disk image (or another drive), so macOS may not keep its permissions. \
             Quit Skerry, drag it into Applications, eject the disk image, and open Skerry from Applications.",
        )
    } else {
        None
    }
}

/// Clear the Accessibility and Input Monitoring entries macOS stored for
/// `bundle_id`.
fn reset_entries(bundle_id: &str) -> Result<()> {
    for service in ["Accessibility", "ListenEvent"] {
        let out = std::process::Command::new("/usr/bin/tccutil").args(["reset", service, bundle_id]).output()?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if service == "Accessibility" {
                anyhow::bail!("macOS refused to reset the Accessibility permission: {err}");
            }
            tracing::info!("tccutil reset {service}: {err}");
        }
    }
    Ok(())
}

/// Forget the input permissions macOS has stored for Skerry and ask again,
/// so the running copy gets approved. Fixes Skerry showing as allowed in
/// System Settings while macOS still blocks it.
pub fn reset_permissions(bundle_id: &str) -> Result<()> {
    reset_entries(bundle_id)?;
    tracing::info!("reset macOS input permissions; asking again");
    accessibility_trusted(true);
    request_input_monitoring();
    Ok(())
}

/// Identifies this copy of the program: its path, size and modification
/// time, which change whenever Skerry is installed or updated.
fn build_stamp() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let meta = std::fs::metadata(&exe).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(format!("{}\n{}\n{modified}\n", exe.display(), meta.len()))
}

/// When this copy of Skerry isn't allowed to use input yet, clear the
/// Accessibility and Input Monitoring entries left by earlier copies, so
/// macOS asks about this one. Done once per installed copy (`marker`
/// remembers which), so someone who declines isn't asked again and again.
///
/// macOS ties each approval to the exact app it was given to. An update of
/// an ad-hoc signed build is a different app, so System Settings keeps
/// showing Skerry switched on while macOS refuses the new copy, and
/// switching it off and on there doesn't help.
pub fn forget_stale_permissions(bundle_id: &str, marker: &Path) {
    if accessibility_trusted(false) || app_bundle().is_none() || location_problem().is_some() {
        return;
    }
    let Some(stamp) = build_stamp() else { return };
    if std::fs::read_to_string(marker).is_ok_and(|s| s == stamp) {
        return;
    }
    match reset_entries(bundle_id) {
        Ok(()) => tracing::info!("cleared permission entries left by an earlier copy of Skerry"),
        Err(e) => tracing::warn!("couldn't clear old permission entries: {e:#}"),
    }
    if let Err(e) = std::fs::write(marker, stamp) {
        tracing::warn!("couldn't write {}: {e}", marker.display());
    }
}

/// Lines for the diagnostics report: how this copy of Skerry is signed,
/// where it runs from, and what macOS currently allows it.
pub fn diagnostics() -> Vec<String> {
    let mut lines = Vec::new();
    match app_bundle() {
        Some(bundle) => {
            lines.push(format!("App: {} ({})", bundle.display(), signature_summary(&bundle)));
            if let Some(problem) = location_problem() {
                lines.push(format!("App location: {problem}"));
            }
        }
        None => lines.push("App: not running from an app bundle".into()),
    }
    let allowed = |ok: bool| if ok { "allowed" } else { "not allowed" };
    lines.push(format!(
        "Accessibility: {}, Input Monitoring: {}",
        allowed(accessibility_trusted(false)),
        allowed(unsafe { CGPreflightListenEventAccess() })
    ));
    lines
}

/// How the app bundle is signed, as macOS's `codesign` reports it.
fn signature_summary(bundle: &Path) -> String {
    let out =
        match std::process::Command::new("/usr/bin/codesign").args(["--display", "--verbose=2"]).arg(bundle).output() {
            Ok(o) => o,
            Err(e) => return format!("couldn't check the signature: {e}"),
        };
    // codesign writes the details to stderr.
    let text = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return format!("not signed: {}", text.trim());
    }
    describe_signature(&text)
}

/// Summarise `codesign --display --verbose=2` output.
fn describe_signature(text: &str) -> String {
    let field =
        |name: &str| text.lines().find_map(|l| l.strip_prefix(name).and_then(|v| v.strip_prefix('='))).map(str::trim);
    let identifier = field("Identifier").unwrap_or("unknown");
    let flags = text.lines().find_map(|l| l.split_once("flags=").map(|(_, f)| f)).unwrap_or("");
    let sealed = text.lines().any(|l| l.starts_with("Sealed Resources version"));
    if flags.contains("linker-signed") || !sealed {
        return format!(
            "only the program is signed, as {identifier}: macOS can't recognise this copy as Skerry, so it \
             never applies its permissions. Install the latest Skerry"
        );
    }
    match field("Authority") {
        Some(authority) => {
            let team = field("TeamIdentifier").filter(|t| *t != "not set");
            match team {
                Some(team) => format!("signed as {identifier} by {authority}, team {team}"),
                None => format!("signed as {identifier} by {authority}"),
            }
        }
        None => format!("signed as {identifier}, ad-hoc: macOS asks for permissions again after updates"),
    }
}

/// Is any mouse button physically down? (Combined session state.)
fn any_button_down() -> bool {
    unsafe { (0..3).any(|b| CGEventSourceButtonState(0, b)) }
}

/// Modifier keys held, from an event's flags (always current, unlike
/// tracking key presses ourselves).
fn mods_from_flags(flags: CGEventFlags) -> Mods {
    let mut m = Mods::NONE;
    if flags.contains(CGEventFlags::CGEventFlagControl) {
        m = m.union(Mods::CTRL);
    }
    if flags.contains(CGEventFlags::CGEventFlagAlternate) {
        m = m.union(Mods::ALT);
    }
    if flags.contains(CGEventFlags::CGEventFlagShift) {
        m = m.union(Mods::SHIFT);
    }
    if flags.contains(CGEventFlags::CGEventFlagCommand) {
        m = m.union(Mods::META);
    }
    m
}

/// True if Skerry may observe and post input events. With `prompt`, macOS
/// shows its permission dialog the first time.
pub fn accessibility_trusted(prompt: bool) -> bool {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let value = if prompt { CFBoolean::true_value() } else { CFBoolean::false_value() };
        let dict = CFDictionary::<CFString, CFType>::from_CFType_pairs(&[(key, value.as_CFType())]);
        AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef() as *const c_void)
    }
}

/// Lets a background app hide the cursor (undocumented WindowServer
/// property, the same one other KVM tools use). Looked up at runtime so a
/// missing symbol only disables cursor hiding.
fn allow_background_cursor_hiding() {
    type DefaultConnection = unsafe extern "C" fn() -> i32;
    type SetProperty = unsafe extern "C" fn(i32, i32, CFStringRef, *const c_void) -> i32;
    unsafe {
        let conn = libc::dlsym(libc::RTLD_DEFAULT, c"_CGSDefaultConnection".as_ptr());
        let set = libc::dlsym(libc::RTLD_DEFAULT, c"CGSSetConnectionProperty".as_ptr());
        if conn.is_null() || set.is_null() {
            return;
        }
        let conn: DefaultConnection = std::mem::transmute(conn);
        let set: SetProperty = std::mem::transmute(set);
        let cid = conn();
        let key = CFString::new("SetsCursorInBackground");
        set(cid, cid, key.as_concrete_TypeRef(), CFBoolean::true_value().as_CFTypeRef());
    }
}

pub fn displays() -> Vec<Rect> {
    CGDisplay::active_displays()
        .unwrap_or_default()
        .into_iter()
        .map(|id| {
            let b = CGDisplay::new(id).bounds();
            Rect::new(
                b.origin.x.round() as i32,
                b.origin.y.round() as i32,
                b.size.width.round() as i32,
                b.size.height.round() as i32,
            )
        })
        .filter(|r| !r.is_empty())
        .collect()
}

pub fn create(tx: CaptureSender) -> Result<Parts> {
    let capture = MacCapture::new(tx);
    let emulation = MacEmulation::new()?;
    Ok((Box::new(capture), Box::new(emulation), Box::new(displays), "macOS".into()))
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

struct Shared {
    tx: CaptureSender,
    edges: AtomicU8,
    block_drag: AtomicBool,
    grabbed: AtomicBool,
    cursor_hidden: AtomicBool,
    buttons: AtomicU8,
    desktop: RwLock<Desktop>,
    hotkeys: Mutex<HotkeyMatcher>,
    local_keys: Mutex<HashSet<u32>>,
    tap_port: AtomicPtr<c_void>,
    status: Mutex<BackendStatus>,
}

pub struct MacCapture {
    shared: Arc<Shared>,
}

impl MacCapture {
    fn new(tx: CaptureSender) -> MacCapture {
        let trusted = accessibility_trusted(true);
        request_input_monitoring();
        let shared = Arc::new(Shared {
            tx,
            edges: AtomicU8::new(0),
            block_drag: AtomicBool::new(true),
            grabbed: AtomicBool::new(false),
            cursor_hidden: AtomicBool::new(false),
            buttons: AtomicU8::new(0),
            desktop: RwLock::new(Desktop::new(displays())),
            hotkeys: Mutex::new(HotkeyMatcher::default()),
            local_keys: Mutex::new(HashSet::new()),
            tap_port: AtomicPtr::new(std::ptr::null_mut()),
            status: Mutex::new(if trusted {
                BackendStatus::Ok
            } else {
                BackendStatus::NeedsPermission(permission_hint())
            }),
        });
        allow_background_cursor_hiding();
        let s = shared.clone();
        std::thread::Builder::new()
            .name("skerry-mac-tap".into())
            .spawn(move || tap_thread(s))
            .expect("spawning event tap thread");
        MacCapture { shared }
    }
}

fn set_status(s: &Shared, status: BackendStatus) {
    let mut cur = s.status.lock().unwrap();
    if *cur != status {
        *cur = status.clone();
        let _ = s.tx.send(CaptureEvent::Status(status));
    }
}

fn tap_thread(s: Arc<Shared>) {
    let events = vec![
        CGEventType::MouseMoved,
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        CGEventType::OtherMouseDown,
        CGEventType::OtherMouseUp,
        CGEventType::LeftMouseDragged,
        CGEventType::RightMouseDragged,
        CGEventType::OtherMouseDragged,
        CGEventType::ScrollWheel,
        CGEventType::KeyDown,
        CGEventType::KeyUp,
        CGEventType::FlagsChanged,
    ];
    loop {
        let cb_shared = s.clone();
        let tap = CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            events.clone(),
            move |_proxy, etype, event| on_event(&cb_shared, etype, event),
        );
        let tap = match tap {
            Ok(t) => t,
            Err(()) => {
                // Not yet allowed; ask again later without prompting.
                set_status(&s, BackendStatus::NeedsPermission(permission_hint()));
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        s.tap_port.store(tap.mach_port().as_concrete_TypeRef() as *mut c_void, Ordering::Release);
        let source = match tap.mach_port().create_runloop_source(0) {
            Ok(src) => src,
            Err(()) => {
                set_status(&s, BackendStatus::Error("cannot attach the event tap".into()));
                return;
            }
        };
        CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
        tap.enable();
        set_status(&s, BackendStatus::Ok);
        CFRunLoop::run_current();
        return;
    }
}

fn button_bit(b: Button) -> u8 {
    match b {
        Button::Left => 1,
        Button::Right => 2,
        Button::Middle => 4,
        Button::Back => 8,
        Button::Forward => 16,
    }
}

fn other_button(n: i64) -> Button {
    match n {
        3 => Button::Back,
        4 => Button::Forward,
        _ => Button::Middle,
    }
}

fn on_event(s: &Shared, etype: CGEventType, event: &CGEvent) -> CallbackResult {
    if matches!(etype, CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput) {
        let port = s.tap_port.load(Ordering::Acquire);
        if !port.is_null() {
            unsafe { CGEventTapEnable(port as CFMachPortRef, true) };
        }
        return CallbackResult::Keep;
    }
    if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == MAGIC {
        return CallbackResult::Keep;
    }
    let grabbed = s.grabbed.load(Ordering::Acquire);

    // Mouse buttons.
    let button = match etype {
        CGEventType::LeftMouseDown => Some((Button::Left, true)),
        CGEventType::LeftMouseUp => Some((Button::Left, false)),
        CGEventType::RightMouseDown => Some((Button::Right, true)),
        CGEventType::RightMouseUp => Some((Button::Right, false)),
        CGEventType::OtherMouseDown | CGEventType::OtherMouseUp => Some((
            other_button(event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER)),
            matches!(etype, CGEventType::OtherMouseDown),
        )),
        _ => None,
    };
    if let Some((b, pressed)) = button {
        let bit = button_bit(b);
        if !grabbed {
            if pressed {
                s.buttons.fetch_or(bit, Ordering::Relaxed);
            } else {
                s.buttons.fetch_and(!bit, Ordering::Relaxed);
            }
            return CallbackResult::Keep;
        }
        if !pressed && s.buttons.fetch_and(!bit, Ordering::Relaxed) & bit != 0 {
            return CallbackResult::Keep; // went down before capture started
        }
        let _ = s.tx.send(CaptureEvent::Button { button: b, pressed });
        return CallbackResult::Drop;
    }

    match etype {
        CGEventType::MouseMoved
        | CGEventType::LeftMouseDragged
        | CGEventType::RightMouseDragged
        | CGEventType::OtherMouseDragged => {
            let dx = event.get_integer_value_field(EventField::MOUSE_EVENT_DELTA_X) as f64;
            let dy = event.get_integer_value_field(EventField::MOUSE_EVENT_DELTA_Y) as f64;
            if grabbed {
                if dx != 0.0 || dy != 0.0 {
                    let _ = s.tx.send(CaptureEvent::Motion { dx, dy });
                }
                return CallbackResult::Drop;
            }
            if maybe_begin(s, event.location(), dx, dy) {
                return CallbackResult::Drop;
            }
            CallbackResult::Keep
        }
        CGEventType::ScrollWheel => {
            if !grabbed {
                return CallbackResult::Keep;
            }
            let continuous = event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS) != 0;
            let (y, x) = if continuous {
                (
                    event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1) * 12,
                    event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2) * 12,
                )
            } else {
                (
                    event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1) * 120,
                    event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2) * 120,
                )
            };
            if x != 0 || y != 0 {
                // macOS reports positive horizontal deltas for leftward scrolling.
                let _ = s.tx.send(CaptureEvent::Scroll { x: -x as i32, y: y as i32 });
            }
            CallbackResult::Drop
        }
        CGEventType::KeyDown | CGEventType::KeyUp | CGEventType::FlagsChanged => {
            let mac = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
            let Some(code) = keymap::to_evdev(mac) else {
                return if grabbed { CallbackResult::Drop } else { CallbackResult::Keep };
            };
            let presses: Vec<bool> = match etype {
                CGEventType::KeyDown => vec![true],
                CGEventType::KeyUp => vec![false],
                _ if code == k::CAPSLOCK => vec![true, false],
                _ => match keymap::modifier_mask(mac) {
                    Some(mask) => vec![event.get_flags().bits() & mask != 0],
                    None => return if grabbed { CallbackResult::Drop } else { CallbackResult::Keep },
                },
            };
            let mut result = if grabbed { CallbackResult::Drop } else { CallbackResult::Keep };
            let mods = mods_from_flags(event.get_flags());
            for pressed in presses {
                match s.hotkeys.lock().unwrap().on_key_with_mods(code, pressed, Some(mods)) {
                    KeyVerdict::Fire(action) => {
                        let _ = s.tx.send(CaptureEvent::Hotkey(action));
                        result = CallbackResult::Drop;
                    }
                    KeyVerdict::Swallow => result = CallbackResult::Drop,
                    KeyVerdict::Pass if !grabbed => {
                        let mut local = s.local_keys.lock().unwrap();
                        if pressed {
                            local.insert(code);
                        } else {
                            local.remove(&code);
                        }
                    }
                    KeyVerdict::Pass => {
                        if !pressed && s.local_keys.lock().unwrap().remove(&code) {
                            result = CallbackResult::Keep;
                            continue;
                        }
                        let _ = s.tx.send(CaptureEvent::Key { code, pressed });
                    }
                }
            }
            result
        }
        _ => CallbackResult::Keep,
    }
}

fn maybe_begin(s: &Shared, at: CGPoint, dx: f64, dy: f64) -> bool {
    let edges = EdgeSet::from_bits(s.edges.load(Ordering::Acquire));
    if edges.is_empty() || (s.block_drag.load(Ordering::Relaxed) && any_button_down()) {
        return false;
    }
    let desk = s.desktop.read().unwrap();
    let (x, y) = desk.clamp(at.x, at.y);
    let Some(edge) = desk.edge_near(x, y, 1.0) else { return false };
    let pushing = match edge {
        Edge::Left => dx < 0.0,
        Edge::Right => dx > 0.0,
        Edge::Top => dy < 0.0,
        Edge::Bottom => dy > 0.0,
    };
    if !edges.contains(edge) || !pushing {
        return false;
    }
    drop(desk);
    start_grab(s);
    let _ = s.tx.send(CaptureEvent::Begin { edge, x, y });
    true
}

fn start_grab(s: &Shared) {
    s.grabbed.store(true, Ordering::Release);
    let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(false);
    if !s.cursor_hidden.swap(true, Ordering::AcqRel) {
        let _ = CGDisplay::main().hide_cursor();
    }
}

fn stop_grab(s: &Shared, warp: Option<(f64, f64)>) {
    s.grabbed.store(false, Ordering::Release);
    if let Some((x, y)) = warp {
        let _ = CGDisplay::warp_mouse_cursor_position(CGPoint::new(x, y));
    }
    let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(true);
    if s.cursor_hidden.swap(false, Ordering::AcqRel) {
        let _ = CGDisplay::main().show_cursor();
    }
}

impl Capture for MacCapture {
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
        start_grab(&self.shared);
    }
    fn release(&self, warp: Option<(f64, f64)>) {
        stop_grab(&self.shared, warp);
    }
    fn displays(&self) -> Vec<Rect> {
        let d = displays();
        *self.shared.desktop.write().unwrap() = Desktop::new(d.clone());
        d
    }
    fn status(&self) -> BackendStatus {
        self.shared.status.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------
// Emulation
// ---------------------------------------------------------------------------

pub struct MacEmulation {
    source: CGEventSource,
    pos: (f64, f64),
    buttons: u8,
    keys: HashSet<u32>,
    last_click: Option<(Button, Instant, (f64, f64), i64)>,
}

// CGEventSource is a CoreFoundation object; it is only ever used from the
// engine's single emulation thread.
unsafe impl Send for MacEmulation {}

impl MacEmulation {
    fn new() -> Result<MacEmulation> {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| anyhow::anyhow!("cannot create a Quartz event source"))?;
        Ok(MacEmulation { source, pos: (0.0, 0.0), buttons: 0, keys: HashSet::new(), last_click: None })
    }

    fn flags(&self) -> CGEventFlags {
        let mut bits = 0u64;
        for code in &self.keys {
            bits |= match *code {
                k::LEFTSHIFT => 0x0002_0000 | 0x02,
                k::RIGHTSHIFT => 0x0002_0000 | 0x04,
                k::LEFTCTRL => 0x0004_0000 | 0x01,
                k::RIGHTCTRL => 0x0004_0000 | 0x2000,
                k::LEFTALT => 0x0008_0000 | 0x20,
                k::RIGHTALT => 0x0008_0000 | 0x40,
                k::LEFTMETA => 0x0010_0000 | 0x08,
                k::RIGHTMETA => 0x0010_0000 | 0x10,
                _ => 0,
            };
        }
        CGEventFlags::from_bits_retain(bits)
    }

    fn post(&self, event: CGEvent) {
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, MAGIC);
        event.post(CGEventTapLocation::HID);
    }

    fn mouse_event(&self, etype: CGEventType, button: CGMouseButton) -> Option<CGEvent> {
        let ev =
            CGEvent::new_mouse_event(self.source.clone(), etype, CGPoint::new(self.pos.0, self.pos.1), button).ok()?;
        ev.set_flags(self.flags());
        Some(ev)
    }
}

impl Emulation for MacEmulation {
    fn motion_abs(&mut self, x: f64, y: f64) {
        let (dx, dy) = (x - self.pos.0, y - self.pos.1);
        self.pos = (x, y);
        let (etype, button) = if self.buttons & 1 != 0 {
            (CGEventType::LeftMouseDragged, CGMouseButton::Left)
        } else if self.buttons & 2 != 0 {
            (CGEventType::RightMouseDragged, CGMouseButton::Right)
        } else if self.buttons != 0 {
            (CGEventType::OtherMouseDragged, CGMouseButton::Center)
        } else {
            (CGEventType::MouseMoved, CGMouseButton::Left)
        };
        if let Some(ev) = self.mouse_event(etype, button) {
            ev.set_integer_value_field(EventField::MOUSE_EVENT_DELTA_X, dx.round() as i64);
            ev.set_integer_value_field(EventField::MOUSE_EVENT_DELTA_Y, dy.round() as i64);
            self.post(ev);
        }
    }

    fn button(&mut self, button: Button, pressed: bool) {
        let bit = button_bit(button);
        if pressed {
            self.buttons |= bit;
        } else {
            self.buttons &= !bit;
        }
        // Click counting for double/triple clicks.
        let count = if pressed {
            let n = match self.last_click {
                Some((b, t, p, n))
                    if b == button
                        && t.elapsed() < Duration::from_millis(500)
                        && (p.0 - self.pos.0).abs() < 5.0
                        && (p.1 - self.pos.1).abs() < 5.0 =>
                {
                    n + 1
                }
                _ => 1,
            };
            self.last_click = Some((button, Instant::now(), self.pos, n));
            n
        } else {
            self.last_click.map(|(_, _, _, n)| n).unwrap_or(1)
        };
        let (etype, cg_button, number) = match (button, pressed) {
            (Button::Left, true) => (CGEventType::LeftMouseDown, CGMouseButton::Left, 0),
            (Button::Left, false) => (CGEventType::LeftMouseUp, CGMouseButton::Left, 0),
            (Button::Right, true) => (CGEventType::RightMouseDown, CGMouseButton::Right, 1),
            (Button::Right, false) => (CGEventType::RightMouseUp, CGMouseButton::Right, 1),
            (b, true) => (CGEventType::OtherMouseDown, CGMouseButton::Center, other_number(b)),
            (b, false) => (CGEventType::OtherMouseUp, CGMouseButton::Center, other_number(b)),
        };
        if let Some(ev) = self.mouse_event(etype, cg_button) {
            ev.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, count);
            ev.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, number);
            self.post(ev);
        }
    }

    fn scroll(&mut self, x: i32, y: i32) {
        let ev = if x % 120 == 0 && y % 120 == 0 {
            CGEvent::new_scroll_event(self.source.clone(), ScrollEventUnit::LINE, 2, y / 120, -x / 120, 0)
        } else {
            CGEvent::new_scroll_event(self.source.clone(), ScrollEventUnit::PIXEL, 2, y / 12, -x / 12, 0)
        };
        if let Ok(ev) = ev {
            ev.set_flags(self.flags());
            self.post(ev);
        }
    }

    fn key(&mut self, code: u32, pressed: bool) {
        let Some(mac) = keymap::from_evdev(code) else { return };
        let repeat = pressed && self.keys.contains(&code);
        if pressed {
            self.keys.insert(code);
        } else {
            self.keys.remove(&code);
        }
        let Ok(ev) = CGEvent::new_keyboard_event(self.source.clone(), mac, pressed) else { return };
        if keymap::modifier_mask(mac).is_some() {
            ev.set_type(CGEventType::FlagsChanged);
        }
        if repeat {
            ev.set_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT, 1);
        }
        ev.set_flags(self.flags());
        self.post(ev);
    }

    fn status(&self) -> BackendStatus {
        if accessibility_trusted(false) {
            BackendStatus::Ok
        } else {
            BackendStatus::NeedsPermission(permission_hint())
        }
    }
}

fn other_number(b: Button) -> i64 {
    match b {
        Button::Back => 3,
        Button::Forward => 4,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::describe_signature;

    #[test]
    fn signature_descriptions() {
        let linker = "Executable=/Applications/Skerry.app/Contents/MacOS/skerry\nIdentifier=skerry-5f1c2a\n\
                      Format=app bundle with Mach-O universal (x86_64 arm64)\n\
                      CodeDirectory v=20400 size=9000 flags=0x20002(adhoc,linker-signed) hashes=278+0 location=embedded\n\
                      Signature=adhoc\nInfo.plist=not bound\nTeamIdentifier=not set\nSealed Resources=none\n";
        assert!(describe_signature(linker).contains("only the program is signed, as skerry-5f1c2a"));

        let adhoc = "Identifier=org.skerry.app\nFormat=app bundle with Mach-O universal (x86_64 arm64)\n\
                     CodeDirectory v=20500 size=9000 flags=0x10002(adhoc,runtime) hashes=278+7 location=embedded\n\
                     Signature=adhoc\nTeamIdentifier=not set\nSealed Resources version=2 rules=13 files=4\n";
        assert!(describe_signature(adhoc).starts_with("signed as org.skerry.app, ad-hoc"));

        let own = "Identifier=org.skerry.app\nCodeDirectory v=20500 size=9000 flags=0x10000(runtime) hashes=278+7\n\
                   Signature size=1678\nAuthority=Skerry Code Signing\nTeamIdentifier=not set\n\
                   Sealed Resources version=2 rules=13 files=4\n";
        assert_eq!(describe_signature(own), "signed as org.skerry.app by Skerry Code Signing");

        let apple = "Identifier=org.skerry.app\nCodeDirectory v=20500 size=9000 flags=0x10000(runtime) hashes=278+7\n\
                     Authority=Developer ID Application: Example (AB12CD34EF)\nAuthority=Developer ID Certification \
                     Authority\nAuthority=Apple Root CA\nTeamIdentifier=AB12CD34EF\n\
                     Sealed Resources version=2 rules=13 files=4\n";
        assert_eq!(
            describe_signature(apple),
            "signed as org.skerry.app by Developer ID Application: Example (AB12CD34EF), team AB12CD34EF"
        );
    }
}
