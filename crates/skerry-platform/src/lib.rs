//! Operating-system specific input capture, input emulation and clipboard
//! access for Skerry.
//!
//! | OS                | capture (control others)            | emulation (be controlled)              |
//! |-------------------|-------------------------------------|----------------------------------------|
//! | Windows 10+       | low-level mouse/keyboard hooks      | `SendInput`                            |
//! | macOS 12+         | Quartz event tap                    | Quartz events (`CGEventPost`)          |
//! | Linux, X11        | XInput2 raw events + grabs          | XTest                                  |
//! | Linux, Wayland    | InputCapture portal + libei         | RemoteDesktop portal + libei, or wlroots virtual pointer/keyboard |

use anyhow::Result;
use skerry_core::clipboard::ClipboardProvider;
use skerry_core::engine::Backends;
use skerry_core::input::{Capture, CaptureEvent, Emulation, ScreenSource};
use skerry_core::keys::OsKind;
use tokio::sync::mpsc;

mod clipboard;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub use clipboard::SystemClipboard;

/// The platform pieces, ready to hand to the engine.
pub struct Platform {
    pub capture: Box<dyn Capture>,
    pub emulation: Box<dyn Emulation>,
    pub screen: Box<dyn ScreenSource>,
    pub clipboard: Box<dyn ClipboardProvider>,
    /// Human-readable name of the backends in use, e.g. "X11".
    pub description: String,
}

/// Create the capture/emulation backends for this computer. Must be called
/// from within a tokio runtime (some backends use async portals).
pub async fn create(events: mpsc::UnboundedSender<CaptureEvent>) -> Result<Platform> {
    let clipboard: Box<dyn ClipboardProvider> = match SystemClipboard::new() {
        Ok(c) => Box::new(c),
        Err(e) => {
            tracing::warn!("clipboard unavailable: {e:#}");
            Box::new(skerry_core::clipboard::NullClipboard)
        }
    };

    #[cfg(target_os = "linux")]
    let (capture, emulation, screen, description) = linux::create(events).await?;
    #[cfg(target_os = "windows")]
    let (capture, emulation, screen, description) = windows::create(events)?;
    #[cfg(target_os = "macos")]
    let (capture, emulation, screen, description) = macos::create(events)?;
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    let (capture, emulation, screen, description): (
        Box<dyn Capture>,
        Box<dyn Emulation>,
        Box<dyn ScreenSource>,
        String,
    ) = {
        let _ = events;
        anyhow::bail!("this operating system is not supported")
    };

    Ok(Platform { capture, emulation, screen, clipboard, description })
}

/// Create the platform and wrap it as engine backends.
pub async fn backends() -> Result<(Backends, String)> {
    let (tx, rx) = mpsc::unbounded_channel();
    let p = create(tx).await?;
    Ok((
        Backends {
            os: OsKind::current(),
            capture: p.capture,
            capture_events: rx,
            emulation: p.emulation,
            screen: p.screen,
            clipboard: p.clipboard,
        },
        p.description,
    ))
}

/// macOS: forget Skerry's stored input permissions and ask for them again.
/// macOS ties an approval to the exact build that was approved, so after an
/// update it can show Skerry as allowed while still blocking it; resetting
/// lets the running copy be approved. Elsewhere this does nothing.
/// `bundle_id` is the app's bundle identifier.
pub fn reset_permissions(bundle_id: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    return macos::reset_permissions(bundle_id);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bundle_id;
        Ok(())
    }
}

/// Process-wide setup that must happen before any window or hook is created
/// (DPI awareness on Windows). Safe to call more than once.
pub fn init_process() {
    #[cfg(target_os = "windows")]
    windows::init_process();
}
