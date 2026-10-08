//! A plain-text troubleshooting report: versions, permissions, network,
//! connections and the most recent log lines.

use skerry_core::engine::{FocusView, Snapshot};
use skerry_core::input::BackendStatus;
use std::fmt::Write;
use std::path::Path;

const LOG_LINES: usize = 200;

fn status(s: &BackendStatus) -> String {
    match s {
        BackendStatus::Ok => "ok".into(),
        BackendStatus::NeedsPermission(d) => format!("needs permission: {d}"),
        BackendStatus::Unsupported(d) => format!("unsupported: {d}"),
        BackendStatus::Error(d) => format!("error: {d}"),
    }
}

pub fn report(s: &Snapshot, backend: &str, log_dir: &Path) -> String {
    let mut r = String::new();
    let name = |id: &str| s.peers.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| id.into());
    let os = match skerry_core::net::macos_version() {
        Some(v) => format!("macOS {v}"),
        None => std::env::consts::OS.to_string(),
    };
    let _ = writeln!(r, "Skerry {} on {os} {} (input: {backend})", s.me.version, std::env::consts::ARCH);
    let _ =
        writeln!(r, "This computer: {} ({:?}), id {}, fingerprint {}", s.me.name, s.me.os, s.me.id, s.me.fingerprint);
    match &s.listen_error {
        Some(e) => {
            let _ = writeln!(r, "Listening: FAILED on port {}: {e}", s.me.port);
        }
        None => {
            let _ = writeln!(r, "Listening: port {}", s.me.port);
        }
    }
    for line in skerry_platform::diagnostics() {
        let _ = writeln!(r, "{line}");
    }
    let _ = writeln!(r, "Capture (control others): {}", status(&s.capture));
    let _ = writeln!(r, "Emulation (be controlled): {}", status(&s.emulation));
    let _ = writeln!(
        r,
        "Sharing: {}, edge switching: {}, clipboard: {}",
        if s.settings.enabled { "on" } else { "paused" },
        if s.settings.edge_switching { "on" } else { "off" },
        if s.settings.clipboard_sync { "on" } else { "off" },
    );
    let focus = match &s.focus {
        FocusView::Local => "local".to_string(),
        FocusView::Controlling(p) => format!("controlling {}", name(p)),
        FocusView::ControlledBy(p) => format!("controlled by {}", name(p)),
    };
    let _ = writeln!(r, "Focus: {focus}");
    let nets: Vec<String> = skerry_core::net::local_ipv4().iter().map(|n| format!("{}/{}", n.ip, n.prefix)).collect();
    let _ = writeln!(r, "Network: {}", if nets.is_empty() { "no IPv4 address".into() } else { nets.join(", ") });
    let fw = crate::firewall::status(s.me.port);
    if fw != "n/a" {
        let _ = writeln!(r, "Windows Firewall: {fw}");
    }
    let side = |e: &Option<String>| e.as_deref().map(name).unwrap_or_else(|| "-".into());
    let _ = writeln!(
        r,
        "Layout: left {}, right {}, top {}, bottom {}",
        side(&s.layout.left),
        side(&s.layout.right),
        side(&s.layout.top),
        side(&s.layout.bottom)
    );

    let _ = writeln!(r, "\nComputers:");
    if s.peers.is_empty() {
        let _ = writeln!(r, "  (none)");
    }
    for p in &s.peers {
        let state = if p.online {
            "online"
        } else if p.available {
            "available"
        } else {
            "offline"
        };
        let _ = writeln!(
            r,
            "  - {} ({:?}, {}) {}{}, address {}, edge {}, id {}",
            p.name,
            p.os,
            p.version.as_deref().unwrap_or("version unknown"),
            if p.paired { "paired, " } else { "" },
            state,
            p.addr.as_deref().unwrap_or("-"),
            p.edge.map(|e| format!("{e:?}").to_lowercase()).unwrap_or_else(|| "-".into()),
            p.id,
        );
        if let Some(e) = &p.last_error {
            let _ = writeln!(r, "      last error: {e}");
        }
    }
    if !s.manual_peers.is_empty() {
        let _ = writeln!(r, "Extra addresses: {}", s.manual_peers.join(", "));
    }

    let _ = writeln!(r, "\nRecent log ({}):", log_dir.display());
    match latest_log(log_dir) {
        Some(text) => {
            let lines: Vec<&str> = text.lines().collect();
            for l in &lines[lines.len().saturating_sub(LOG_LINES)..] {
                let _ = writeln!(r, "{l}");
            }
        }
        None => {
            let _ = writeln!(r, "(no log file yet)");
        }
    }
    r
}

fn latest_log(dir: &Path) -> Option<String> {
    let newest = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("skerry"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max()?
        .1;
    let bytes = std::fs::read(newest).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}
