//! Interfaces implemented by the platform input backends (see the
//! `skerry-platform` crate) and used by the engine.

use serde::Serialize;
use tokio::sync::mpsc::UnboundedSender;

use crate::geometry::{Edge, EdgeSet, Rect};
use crate::keys::{Hotkey, HotkeyAction};
use crate::proto::Button;

/// Events from the capture backend of the computer whose mouse and keyboard
/// are physically in use.
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureEvent {
    /// The cursor was pushed through an enabled edge at `(x, y)` (capture
    /// coordinates). The backend has started capturing: local input is now
    /// hidden from this computer and reported below until `release`.
    Begin {
        edge: Edge,
        x: f64,
        y: f64,
    },
    /// Relative pointer movement while capturing.
    Motion {
        dx: f64,
        dy: f64,
    },
    Button {
        button: Button,
        pressed: bool,
    },
    /// Scroll in 1/120ths of a notch; positive y = up, positive x = right.
    Scroll {
        x: i32,
        y: i32,
    },
    /// Key in evdev code space, while capturing.
    Key {
        code: u32,
        pressed: bool,
    },
    /// A configured hotkey was pressed (captured or not).
    Hotkey(HotkeyAction),
    /// The backend's health changed (e.g. a permission is missing).
    Status(BackendStatus),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum BackendStatus {
    Ok,
    /// Works only after the user grants a permission (detail says which).
    NeedsPermission(String),
    /// Not available on this system (detail says why).
    Unsupported(String),
    Error(String),
}

impl BackendStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, BackendStatus::Ok)
    }
}

pub type CaptureSender = UnboundedSender<CaptureEvent>;

/// Watches the local cursor and, once it crosses an enabled edge, captures
/// all mouse and keyboard input until released.
///
/// Implementations run their own OS event loop thread; these methods only
/// signal it and must return quickly.
pub trait Capture: Send + Sync {
    /// Edges that currently lead to another computer.
    fn set_edges(&self, edges: EdgeSet);
    /// Hotkeys to detect. Matching keys are hidden from the local system.
    fn set_hotkeys(&self, hotkeys: Vec<(Hotkey, HotkeyAction)>);
    /// Whether to refuse edge switching while a mouse button is held.
    fn set_block_while_dragging(&self, _block: bool) {}
    /// Start capturing without an edge crossing (hotkey switch).
    fn grab(&self);
    /// Stop capturing; optionally warp the cursor to `warp` first.
    fn release(&self, warp: Option<(f64, f64)>);
    /// Displays in capture coordinates.
    fn displays(&self) -> Vec<Rect>;
    fn status(&self) -> BackendStatus {
        BackendStatus::Ok
    }
    /// True if edge crossings and hotkeys come only from this computer's own
    /// mouse and keyboard, never from input Skerry replays for another
    /// computer. Then the local mouse and keyboard can take over while
    /// another computer is in control.
    fn local_input_only(&self) -> bool {
        false
    }
}

/// Replays input received from another computer.
pub trait Emulation: Send {
    fn motion_abs(&mut self, x: f64, y: f64);
    fn button(&mut self, button: Button, pressed: bool);
    fn scroll(&mut self, x: i32, y: i32);
    fn key(&mut self, code: u32, pressed: bool);
    /// True if the OS generates key repeat on its own for held injected keys,
    /// in which case repeated presses from the sender are dropped.
    fn os_key_repeat(&self) -> bool {
        false
    }
    fn status(&self) -> BackendStatus {
        BackendStatus::Ok
    }
}

/// Provides the display layout this computer advertises to others, in the
/// coordinate space `Emulation::motion_abs` expects.
pub trait ScreenSource: Send + Sync {
    fn displays(&self) -> Vec<Rect>;
}

impl<F: Fn() -> Vec<Rect> + Send + Sync> ScreenSource for F {
    fn displays(&self) -> Vec<Rect> {
        self()
    }
}

/// Capture backend for systems where capturing is not possible. The
/// computer can still be controlled by others.
pub struct NullCapture {
    pub displays: Vec<Rect>,
    pub status: BackendStatus,
}

impl Capture for NullCapture {
    fn set_edges(&self, _: EdgeSet) {}
    fn set_hotkeys(&self, _: Vec<(Hotkey, HotkeyAction)>) {}
    fn grab(&self) {}
    fn release(&self, _: Option<(f64, f64)>) {}
    fn displays(&self) -> Vec<Rect> {
        self.displays.clone()
    }
    fn status(&self) -> BackendStatus {
        self.status.clone()
    }
}

pub struct NullEmulation {
    pub status: BackendStatus,
}

impl Emulation for NullEmulation {
    fn motion_abs(&mut self, _: f64, _: f64) {}
    fn button(&mut self, _: Button, _: bool) {}
    fn scroll(&mut self, _: i32, _: i32) {}
    fn key(&mut self, _: u32, _: bool) {}
    fn status(&self) -> BackendStatus {
        self.status.clone()
    }
}
