//! macOS "Start at login".
//!
//! A launchd agent that opens Skerry through LaunchServices
//! (`open -b <bundle id>`), exactly as if the user had opened the app, and
//! names the app in `AssociatedBundleIdentifiers`. macOS then attributes
//! Skerry's permissions and Local Network access to the app.
//!
//! The autostart plugin's agent started the executable directly and named no
//! app. macOS can't tell which app such an agent belongs to (Apple TN3179),
//! so Skerry started at login could have its local network connections
//! blocked ("No route to host") without macOS ever asking.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::PathBuf;

/// Same file the autostart plugin used, so older agents get replaced.
fn agent_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/LaunchAgents/Skerry.plist"))
}

fn agent_plist(bundle_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>Skerry</string>
  <key>AssociatedBundleIdentifiers</key>
  <array>
    <string>{bundle_id}</string>
  </array>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>-g</string>
    <string>-b</string>
    <string>{bundle_id}</string>
    <string>--args</string>
    <string>--minimized</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
    )
}

pub fn is_enabled() -> bool {
    agent_path().is_some_and(|p| p.exists())
}

pub fn set(enabled: bool, bundle_id: &str) -> std::io::Result<()> {
    let path = agent_path().ok_or_else(|| std::io::Error::other("no home directory"))?;
    if enabled {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, agent_plist(bundle_id))
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// Replace an agent written by an older version with the current one.
pub fn upgrade(bundle_id: &str) {
    let Some(path) = agent_path() else { return };
    let Ok(old) = std::fs::read_to_string(&path) else { return };
    let new = agent_plist(bundle_id);
    if old != new {
        match std::fs::write(&path, new) {
            Ok(()) => tracing::info!("updated the Start at login agent"),
            Err(e) => tracing::warn!("couldn't update the Start at login agent: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn agent_opens_the_app_and_names_it() {
        let p = super::agent_plist("org.skerry.app");
        assert!(p.contains("<key>AssociatedBundleIdentifiers</key>\n  <array>\n    <string>org.skerry.app</string>"));
        assert!(p.contains("<string>/usr/bin/open</string>"));
        assert!(p.contains("<string>--minimized</string>"));
    }
}
