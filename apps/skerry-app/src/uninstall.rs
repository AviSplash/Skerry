//! "Uninstall Skerry": removes the app and everything it stored on this
//! computer.
//!
//! The steps that can fail or need the user's approval run first, while
//! Skerry is still open and can report a problem: removing the Linux package
//! or AppImage, deleting the macOS app bundle, and removing Windows Firewall
//! rules. If one of those fails, nothing else is touched.
//!
//! Then Skerry stops, removes its start-at-login entry and quits, and a small
//! script finishes the job once the process has exited (Windows can't delete
//! files that are still open): settings, pairing keys, logs, web view data
//! and caches, macOS privacy entries, and on Windows the program itself,
//! through its uninstaller.
//!
//! As a guard against deleting anything else, a path is only ever removed if
//! it contains "skerry".

use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// How the program itself is removed.
#[derive(Debug, Clone, PartialEq)]
pub enum AppRemoval {
    /// macOS: delete the app bundle.
    MacBundle(PathBuf),
    /// Linux: delete the AppImage file.
    AppImage(PathBuf),
    /// Linux: remove the package (needs the user's password, via pkexec).
    Package { manager: PackageManager, name: String },
    /// Windows: run Skerry's uninstaller (NSIS or MSI) after quitting.
    WindowsUninstaller,
    /// Not installed in a way Skerry recognises, e.g. a development build.
    Manual(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PackageManager {
    Deb,
    Rpm,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub app: AppRemoval,
    /// Deleted (recursively) after Skerry has quit.
    pub paths: Vec<PathBuf>,
    /// Removed after that, but only if empty.
    pub if_empty: Vec<PathBuf>,
    /// Windows Firewall rules for Skerry exist and need removing.
    pub firewall: bool,
}

/// What the confirmation dialog shows.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub items: Vec<String>,
    pub paths: Vec<String>,
    pub notes: Vec<String>,
}

/// Only ever delete paths that are clearly Skerry's: absolute, not right
/// below the root, and named after Skerry.
pub fn is_ours(p: &Path) -> bool {
    let depth = p.components().filter(|c| matches!(c, std::path::Component::Normal(_))).count();
    p.is_absolute() && depth >= 2 && p.to_string_lossy().to_lowercase().contains("skerry")
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

pub fn plan(app: &AppHandle, config_dir: &Path) -> Plan {
    let id = app.config().identifier.clone();
    let mut paths = Vec::new();
    let mut if_empty = Vec::new();

    // Skerry's own settings, pairing keys and logs.
    if std::env::var_os("SKERRY_CONFIG_DIR").is_some() {
        // A custom folder may hold other things: only remove Skerry's files.
        for f in ["config.toml", "identity.key", "logs"] {
            paths.push(config_dir.join(f));
        }
        if_empty.push(config_dir.to_path_buf());
    } else {
        paths.push(config_dir.to_path_buf());
        // Windows keeps it in %APPDATA%\Skerry\Skerry\config.
        if cfg!(windows) {
            if let Some(app_dir) = config_dir.parent().filter(|p| p.ends_with("Skerry")) {
                paths.push(app_dir.to_path_buf());
                if let Some(org) = app_dir.parent() {
                    if_empty.push(org.to_path_buf());
                }
            }
        }
    }

    // The desktop shell's data: web view storage, caches, logs.
    let r = app.path();
    paths.extend(
        [r.app_config_dir(), r.app_data_dir(), r.app_local_data_dir(), r.app_cache_dir(), r.app_log_dir()]
            .into_iter()
            .flatten(),
    );

    if let Some(home) = home() {
        if cfg!(target_os = "macos") {
            let lib = home.join("Library");
            paths.push(lib.join("WebKit").join(&id));
            paths.push(lib.join("HTTPStorages").join(&id));
            paths.push(lib.join("HTTPStorages").join(format!("{id}.binarycookies")));
            paths.push(lib.join("Saved Application State").join(format!("{id}.savedState")));
            paths.push(lib.join("Preferences").join(format!("{id}.plist")));
            paths.push(lib.join("LaunchAgents").join("Skerry.plist"));
        }
        if cfg!(target_os = "linux") {
            let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
            paths.push(config.join("autostart").join("Skerry.desktop"));
        }
    }

    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| {
        if !is_ours(p) {
            tracing::warn!("uninstall: not removing {} (doesn't look like Skerry's)", p.display());
            return false;
        }
        seen.insert(p.clone())
    });
    if_empty.retain(|p| is_ours(p));

    Plan { app: app_removal(&id), paths, if_empty, firewall: crate::firewall::has_rules() }
}

fn app_removal(bundle_id: &str) -> AppRemoval {
    let exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(p) => p,
        Err(e) => return AppRemoval::Manual(format!("Couldn't find Skerry's program file ({e}); delete it yourself.")),
    };
    let manual = || {
        AppRemoval::Manual(format!(
            "This copy of Skerry ({}) wasn't installed from a Skerry installer, so delete it yourself.",
            exe.display()
        ))
    };

    if cfg!(target_os = "macos") {
        // …/Skerry.app/Contents/MacOS/skerry
        let Some(bundle) = exe.ancestors().nth(3) else { return manual() };
        let is_bundle = bundle.extension().is_some_and(|e| e == "app")
            && std::fs::read_to_string(bundle.join("Contents/Info.plist")).is_ok_and(|s| s.contains(bundle_id));
        if !is_bundle {
            return manual();
        }
        if bundle.to_string_lossy().contains("/AppTranslocation/") {
            return AppRemoval::Manual(
                "macOS is running Skerry from a temporary copy, so drag Skerry from the folder you opened it from to \
                 the Trash."
                    .into(),
            );
        }
        return AppRemoval::MacBundle(bundle.to_path_buf());
    }

    use tauri::utils::config::BundleType;
    match tauri::utils::platform::bundle_type() {
        Some(BundleType::Nsis | BundleType::Msi) => AppRemoval::WindowsUninstaller,
        Some(BundleType::AppImage) => match std::env::var_os("APPIMAGE").map(PathBuf::from) {
            Some(p) if p.is_file() => AppRemoval::AppImage(p),
            _ => manual(),
        },
        Some(BundleType::Deb) => package_owning(&exe, PackageManager::Deb).unwrap_or_else(manual),
        Some(BundleType::Rpm) => package_owning(&exe, PackageManager::Rpm).unwrap_or_else(manual),
        _ => manual(),
    }
}

/// The installed package that owns `file`.
fn package_owning(file: &Path, manager: PackageManager) -> Option<AppRemoval> {
    let out = match manager {
        PackageManager::Deb => std::process::Command::new("dpkg-query").arg("-S").arg(file).output(),
        PackageManager::Rpm => {
            std::process::Command::new("rpm").args(["-qf", "--queryformat", "%{NAME}"]).arg(file).output()
        }
    }
    .ok()
    .filter(|o| o.status.success())?;
    let name = package_name(&String::from_utf8_lossy(&out.stdout))?;
    Some(AppRemoval::Package { manager, name })
}

/// dpkg-query prints "skerry: /usr/bin/skerry"; rpm prints just the name.
fn package_name(output: &str) -> Option<String> {
    let name = output.lines().next()?.split(':').next()?.trim().to_string();
    let valid = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "+-._".contains(c));
    valid.then_some(name)
}

pub fn summary(plan: &Plan) -> Summary {
    let mut items = Vec::new();
    let mut notes = Vec::new();
    match &plan.app {
        AppRemoval::MacBundle(p) => items.push(format!("The Skerry app ({})", p.display())),
        AppRemoval::AppImage(p) => items.push(format!("The Skerry AppImage ({})", p.display())),
        AppRemoval::Package { manager, name } => items.push(format!(
            "The Skerry package “{name}” ({}); Linux asks for your password",
            if *manager == PackageManager::Deb { "deb" } else { "rpm" }
        )),
        AppRemoval::WindowsUninstaller => {
            items.push("The Skerry program, its shortcuts and its entry in Apps & features".into())
        }
        AppRemoval::Manual(why) => notes.push(why.clone()),
    }
    items.push("Your Skerry settings, pairings and this computer's Skerry key".into());
    items.push("Skerry's logs, saved window data and caches".into());
    items.push("Skerry's “Start at login” entry".into());
    if cfg!(target_os = "macos") {
        items.push("Skerry's entries under Privacy & Security → Accessibility and Input Monitoring".into());
        notes.push(
            "macOS doesn't let apps remove themselves from Privacy & Security → Local Network. The entry stays there \
             but does nothing once Skerry is gone."
                .into(),
        );
    }
    if plan.firewall {
        items.push("Skerry's Windows Firewall rules; Windows asks for administrator approval".into());
    }
    notes.push("Your other computers keep this one in their list until you click Forget on them.".into());
    Summary { items, paths: plan.paths.iter().map(|p| p.display().to_string()).collect(), notes }
}

/// Remove the program itself (and on Windows, the firewall rules) while
/// Skerry can still report a problem. Nothing else has been removed if this
/// fails.
pub fn remove_app_first(plan: &Plan) -> Result<(), String> {
    match &plan.app {
        AppRemoval::MacBundle(p) => std::fs::remove_dir_all(p).map_err(|e| {
            format!("Couldn't delete {}: {e}. Nothing was removed. Drag Skerry to the Trash instead.", p.display())
        })?,
        AppRemoval::AppImage(p) => std::fs::remove_file(p)
            .map_err(|e| format!("Couldn't delete {}: {e}. Nothing was removed.", p.display()))?,
        AppRemoval::Package { manager, name } => {
            let (tool, flag) = match manager {
                PackageManager::Deb => ("dpkg", "-r"),
                PackageManager::Rpm => ("rpm", "-e"),
            };
            let manual = format!("To remove it yourself, run: sudo {tool} {flag} {name}");
            let status = std::process::Command::new("pkexec").args([tool, flag, name]).status();
            match status {
                Ok(s) if s.success() => tracing::info!("removed package {name}"),
                // 126: the password prompt was dismissed; 127: not authorised.
                Ok(s) if matches!(s.code(), Some(126 | 127)) => {
                    return Err("Uninstall cancelled: removing Skerry's package needs your password. Nothing was \
                                removed."
                        .into())
                }
                Ok(s) => {
                    return Err(format!("Removing the {name} package failed ({s}). Nothing else was removed. {manual}"))
                }
                Err(e) => return Err(format!("Couldn't ask for your password ({e}). Nothing was removed. {manual}")),
            }
        }
        AppRemoval::WindowsUninstaller | AppRemoval::Manual(_) => {}
    }
    if plan.firewall {
        crate::firewall::remove_rules()?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The cleanup script that runs after Skerry quits
// ---------------------------------------------------------------------------

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn ps_quote(s: &str) -> String {
    // PowerShell also treats typographic single quotes as quotes.
    let mut out = String::from("'");
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// The script for this OS. `system` adds the steps that touch the OS
/// beyond the listed paths (privacy entries, registry, uninstaller); tests
/// leave it off.
pub fn cleanup_script(plan: &Plan, pid: u32, bundle_id: &str, system: bool) -> String {
    if cfg!(windows) {
        windows_script(plan, pid, system)
    } else {
        unix_script(plan, pid, bundle_id, system && cfg!(target_os = "macos"))
    }
}

/// `mac_system`: also clear macOS privacy entries and preferences.
fn unix_script(plan: &Plan, pid: u32, bundle_id: &str, mac_system: bool) -> String {
    let mut s = String::from("#!/bin/sh\n# Finishes uninstalling Skerry once it has quit. Written by Skerry.\n");
    s += &format!("pid={pid}\nn=0\n");
    s += "while kill -0 \"$pid\" 2>/dev/null && [ \"$n\" -lt 150 ]; do sleep 0.2; n=$((n + 1)); done\n";
    if mac_system {
        let id = sh_quote(bundle_id);
        for service in ["Accessibility", "ListenEvent", "PostEvent"] {
            s += &format!("/usr/bin/tccutil reset {service} {id} >/dev/null 2>&1\n");
        }
        s += &format!("/usr/bin/defaults delete {id} >/dev/null 2>&1\n");
        s += &format!("rm -f /tmp/{}*_si.sock\n", bundle_id.replace(['.', '-'], "_"));
    }
    for p in &plan.paths {
        s += &format!("rm -rf -- {}\n", sh_quote(&p.to_string_lossy()));
    }
    for p in &plan.if_empty {
        s += &format!("rmdir -- {} 2>/dev/null\n", sh_quote(&p.to_string_lossy()));
    }
    s += "rm -f -- \"$0\"\n";
    s
}

fn windows_script(plan: &Plan, pid: u32, system: bool) -> String {
    let mut s = String::from("# Finishes uninstalling Skerry once it has quit. Written by Skerry.\n");
    s += "$ErrorActionPreference = 'SilentlyContinue'\n";
    s += &format!("Wait-Process -Id {pid} -Timeout 30\n");
    if system && plan.app == AppRemoval::WindowsUninstaller {
        s += r#"$keys = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
  'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
  'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*')
foreach ($e in @(Get-ItemProperty -Path $keys | Where-Object { $_.DisplayName -eq 'Skerry' })) {
  if ($e.WindowsInstaller -eq 1) {
    Start-Process -FilePath msiexec.exe -ArgumentList @('/x', $e.PSChildName, '/passive', '/norestart') -Wait
  } elseif ($e.UninstallString) {
    $exe = $e.UninstallString.Trim('"')
    $dir = Split-Path -Parent $exe
    if (Test-Path -LiteralPath $exe) {
      # _?= makes the uninstaller run in place, so this waits for it.
      Start-Process -FilePath $exe -ArgumentList @('/S', "_?=$dir") -Wait
      Remove-Item -LiteralPath $exe -Force
    }
    if ($dir -match 'skerry' -and (Test-Path -LiteralPath $dir)) { Remove-Item -LiteralPath $dir -Recurse -Force }
    Remove-Item -LiteralPath $e.PSPath -Recurse -Force
  }
}
"#;
    }
    if system {
        s += r#"Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Skerry'
Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run' -Name 'Skerry'
Remove-Item -Path 'HKCU:\Software\Skerry contributors\Skerry' -Recurse -Force
if (-not (Get-ChildItem -Path 'HKCU:\Software\Skerry contributors')) { Remove-Item -Path 'HKCU:\Software\Skerry contributors' -Force }
"#;
    }
    let list = |v: &[PathBuf]| v.iter().map(|p| ps_quote(&p.to_string_lossy())).collect::<Vec<_>>().join(", ");
    // Web view helper processes can hold files for a moment after Skerry exits.
    s += &format!(
        "foreach ($p in @({})) {{\n  for ($i = 0; $i -lt 20 -and (Test-Path -LiteralPath $p); $i++) {{\n    Remove-Item -LiteralPath $p -Recurse -Force\n    if (Test-Path -LiteralPath $p) {{ Start-Sleep -Milliseconds 500 }}\n  }}\n}}\n",
        list(&plan.paths)
    );
    s += &format!(
        "foreach ($p in @({})) {{\n  if ((Test-Path -LiteralPath $p) -and -not (Get-ChildItem -LiteralPath $p -Force)) {{ Remove-Item -LiteralPath $p -Force }}\n}}\n",
        list(&plan.if_empty)
    );
    s += "Remove-Item -LiteralPath $PSCommandPath -Force\n";
    s
}

/// Write the cleanup script and start it detached, so it outlives Skerry.
pub fn start_cleanup(script: &str) -> std::io::Result<()> {
    let pid = std::process::id();
    let path =
        std::env::temp_dir().join(format!("skerry-uninstall-{pid}.{}", if cfg!(windows) { "ps1" } else { "sh" }));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Windows PowerShell reads a script as UTF-8 only with a byte order mark.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(script.as_bytes());
        std::fs::write(&path, bytes)?;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
            .arg(&path)
            .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
            .spawn()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        use std::process::Stdio;
        std::fs::write(&path, script)?;
        std::process::Command::new("/bin/sh")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_only_accepts_skerry_paths() {
        let abs = |s: &str| if cfg!(windows) { PathBuf::from(format!("C:{s}")) } else { PathBuf::from(s) };
        assert!(is_ours(&abs("/home/ana/.config/skerry")));
        assert!(is_ours(&abs("/Users/ana/Library/Caches/org.skerry.app")));
        assert!(!is_ours(&abs("/home/ana")));
        assert!(!is_ours(&abs("/home/ana/.config")));
        assert!(!is_ours(&abs("/skerry")), "too close to the root");
        assert!(!is_ours(&abs("/home")));
        assert!(!is_ours(Path::new("skerry/config.toml")), "relative");
    }

    #[test]
    fn package_names() {
        assert_eq!(package_name("skerry: /usr/bin/skerry\n").as_deref(), Some("skerry"));
        assert_eq!(package_name("skerry").as_deref(), Some("skerry"));
        assert_eq!(package_name("").as_deref(), None);
        assert_eq!(package_name("evil; rm -rf /: /usr/bin/skerry").as_deref(), None);
    }

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("/a b/it's"), r"'/a b/it'\''s'");
        assert_eq!(ps_quote(r"C:\Users\O'Brien"), r"'C:\Users\O''Brien'");
        assert_eq!(ps_quote("a\u{2019}b"), "'a\u{2019}\u{2019}b'");
    }

    /// Runs the real script on throwaway files: listed paths go, everything
    /// else stays, and the script removes itself.
    #[test]
    fn cleanup_script_removes_only_listed_paths() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("skerry test's dir");
        let config = base.join("config");
        let cache = base.join("org.skerry.app");
        let org = base.join("empty-skerry-org");
        let keep = base.join("keep-skerry-unlisted");
        for d in [&config, &cache, &org, &keep] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(config.join("identity.key"), "secret").unwrap();
        std::fs::write(cache.join("c"), "x").unwrap();
        std::fs::write(keep.join("k"), "x").unwrap();
        let plan = Plan {
            app: AppRemoval::Manual(String::new()),
            paths: vec![config.clone(), cache.clone(), base.join("missing-skerry")],
            if_empty: vec![org.clone(), keep.clone()],
            firewall: false,
        };
        // A process that has already exited stands in for Skerry.
        let mut child = if cfg!(windows) {
            std::process::Command::new("cmd").args(["/C", "exit"]).spawn().unwrap()
        } else {
            std::process::Command::new("true").spawn().unwrap()
        };
        let pid = child.id();
        child.wait().unwrap();

        let script = cleanup_script(&plan, pid, "org.skerry.test", false);
        let file = base.join(if cfg!(windows) { "cleanup.ps1" } else { "cleanup.sh" });
        std::fs::write(&file, &script).unwrap();
        let status = if cfg!(windows) {
            std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(&file)
                .status()
        } else {
            std::process::Command::new("/bin/sh").arg(&file).status()
        }
        .unwrap();
        assert!(status.success(), "script failed:\n{script}");
        assert!(!config.exists() && !cache.exists(), "listed paths removed");
        assert!(!org.exists(), "empty folder removed");
        assert!(keep.join("k").exists(), "a non-empty folder is kept");
        assert!(!file.exists(), "the script removes itself");
    }
}
