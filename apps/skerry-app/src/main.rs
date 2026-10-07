//! Skerry desktop app: the engine plus a settings window and a tray icon.

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod diagnostics;
mod firewall;
mod login_item;

use serde::Serialize;
use skerry_core::clipboard::NullClipboard;
use skerry_core::config::Paths;
use skerry_core::engine::{
    self, Backends, EngineEvent, EngineHandle, EngineOptions, FocusView, PairTarget, SettingsUpdate, Snapshot,
};
use skerry_core::geometry::Edge;
use skerry_core::input::{BackendStatus, NullCapture, NullEmulation};
use skerry_core::keys::OsKind;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, UserAttentionType, WindowEvent, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_updater::{Update, UpdaterExt};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// First automatic update check after start, then how often to check again.
const FIRST_UPDATE_CHECK: Duration = Duration::from_secs(20);
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

struct AppState {
    engine: EngineHandle,
    backend: String,
    config_path: String,
    log_dir: PathBuf,
}

/// The newest update found by the last check, ready to install.
#[derive(Default)]
struct Updates(Mutex<Option<Update>>);

/// Tray menu entry for updates, relabelled when one is available.
struct UpdateMenuItem(MenuItem<Wry>);

#[derive(Serialize)]
struct Info {
    backend: String,
    config_path: String,
    log_dir: String,
    version: String,
}

#[derive(Serialize, Clone)]
struct UpdateInfo {
    version: String,
    current: String,
    notes: Option<String>,
}

#[derive(Serialize, Clone)]
struct UpdateProgress {
    downloaded: usize,
    total: Option<u64>,
}

#[tauri::command]
fn get_state(state: State<'_, AppState>) -> Snapshot {
    state.engine.snapshot()
}

#[tauri::command]
fn get_info(state: State<'_, AppState>) -> Info {
    Info {
        backend: state.backend.clone(),
        config_path: state.config_path.clone(),
        log_dir: state.log_dir.display().to_string(),
        version: skerry_core::APP_VERSION.to_string(),
    }
}

#[tauri::command]
async fn pair(state: State<'_, AppState>, target: PairTarget) -> Result<u64, String> {
    state.engine.pair(target).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn submit_code(state: State<'_, AppState>, session: u64, code: String) {
    state.engine.submit_code(session, &code);
}

#[tauri::command]
fn cancel_pairing(state: State<'_, AppState>, session: u64) {
    state.engine.cancel_pairing(session);
}

#[tauri::command]
fn set_layout(state: State<'_, AppState>, edge: Edge, peer: Option<String>) {
    state.engine.set_layout(edge, peer);
}

#[tauri::command]
fn forget(state: State<'_, AppState>, id: String) {
    state.engine.forget(&id);
}

#[tauri::command]
fn update_settings(state: State<'_, AppState>, settings: SettingsUpdate) {
    state.engine.update_settings(settings);
}

#[tauri::command]
fn set_speed(state: State<'_, AppState>, id: String, speed: f64) {
    state.engine.set_speed(&id, speed);
}

#[tauri::command]
fn add_manual_peer(state: State<'_, AppState>, addr: String) {
    state.engine.add_manual_peer(&addr);
}

#[tauri::command]
fn remove_manual_peer(state: State<'_, AppState>, addr: String) {
    state.engine.remove_manual_peer(&addr);
}

/// Look for computers on the network again.
#[tauri::command]
fn rescan(state: State<'_, AppState>) {
    state.engine.rescan();
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    if cfg!(target_os = "macos") {
        return login_item::is_enabled();
    }
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        return login_item::set(enabled, &app.config().identifier).map_err(|e| e.to_string());
    }
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())
}

/// Open the OS page where the user grants a permission: "accessibility"
/// (default), "input_monitoring" or "local_network".
#[tauri::command]
fn open_permission_settings(pane: Option<String>) {
    #[cfg(target_os = "macos")]
    {
        let anchor = match pane.as_deref() {
            Some("input_monitoring") => "Privacy_ListenEvent",
            Some("local_network") => "Privacy_LocalNetwork",
            _ => "Privacy_Accessibility",
        };
        let _ = std::process::Command::new("open")
            .arg(format!("x-apple.systempreferences:com.apple.preference.security?{anchor}"))
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = pane;
}

/// macOS: clear the approval macOS stored for an earlier copy of Skerry and
/// ask again, for when Skerry shows as allowed but macOS still blocks it.
#[tauri::command]
async fn reset_permissions(app: AppHandle) -> Result<(), String> {
    let bundle_id = app.config().identifier.clone();
    tauri::async_runtime::spawn_blocking(move || skerry_platform::reset_permissions(&bundle_id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

/// A report about this computer, its connections and recent log lines, for
/// troubleshooting and bug reports.
#[tauri::command]
async fn get_diagnostics(state: State<'_, AppState>) -> Result<String, String> {
    let snapshot = state.engine.snapshot();
    let (backend, log_dir) = (state.backend.clone(), state.log_dir.clone());
    tauri::async_runtime::spawn_blocking(move || diagnostics::report(&snapshot, &backend, &log_dir))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_logs(state: State<'_, AppState>) -> Result<(), String> {
    let _ = std::fs::create_dir_all(&state.log_dir);
    open_path(&state.log_dir).map_err(|e| e.to_string())
}

fn open_path(path: &std::path::Path) -> std::io::Result<()> {
    let opener = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener).arg(path).spawn().map(|_| ())
}

/// Whether Windows Firewall lets other computers connect to Skerry:
/// "ok", "blocked", "missing", "unknown", or "n/a" on other systems.
#[tauri::command]
async fn firewall_status(state: State<'_, AppState>) -> Result<String, String> {
    let port = state.engine.snapshot().me.port;
    tauri::async_runtime::spawn_blocking(move || firewall::status(port)).await.map_err(|e| e.to_string())
}

/// Add a Windows Firewall rule that allows Skerry (asks for administrator
/// approval), then report the new status.
#[tauri::command]
async fn fix_firewall(state: State<'_, AppState>) -> Result<String, String> {
    let port = state.engine.snapshot().me.port;
    tauri::async_runtime::spawn_blocking(move || {
        firewall::allow()?;
        Ok(firewall::status(port))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    find_update(&app).await
}

/// Download the update found earlier, install it and restart Skerry.
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let pending = app.state::<Updates>().0.lock().unwrap().clone();
    let update = match pending {
        Some(u) => u,
        None => {
            let updater = app.updater().map_err(|e| e.to_string())?;
            match updater.check().await.map_err(check_error)? {
                Some(u) => u,
                None => return Err("Skerry is already up to date.".into()),
            }
        }
    };
    tracing::info!("downloading Skerry {}", update.version);
    let progress = app.clone();
    let mut downloaded = 0usize;
    let mut last_percent = None;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk;
                let percent = total.map(|t| downloaded as u64 * 100 / t.max(1));
                if percent != last_percent || total.is_none() {
                    last_percent = percent;
                    let _ = progress.emit("update-progress", UpdateProgress { downloaded, total });
                }
            },
            || {},
        )
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    tracing::info!("installing Skerry {}", update.version);
    let engine = app.state::<AppState>().engine.clone();
    // On Windows the installer replaces the running program and the call
    // below exits, so let other computers know first.
    #[cfg(target_os = "windows")]
    engine.shutdown().await;
    if let Err(e) = update.install(bytes) {
        tracing::error!("installing the update failed: {e}");
        #[cfg(target_os = "windows")]
        app.restart();
        #[cfg(not(target_os = "windows"))]
        return Err(format!("Installing the update failed: {e}"));
    }
    engine.shutdown().await;
    app.restart();
}

fn check_error(e: tauri_plugin_updater::Error) -> String {
    tracing::info!("update check failed: {e}");
    let raw = e.to_string();
    if raw.contains("valid release JSON") || raw.contains("404") {
        "Couldn't get update information. Check the internet connection, or download the latest version from \
         github.com/AviSplash/Skerry/releases."
            .into()
    } else if raw.contains("platforms") {
        "Automatic updates aren't available for this kind of installation. Download the latest version from \
         github.com/AviSplash/Skerry/releases."
            .into()
    } else {
        format!("Couldn't check for updates: {raw}")
    }
}

async fn find_update(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(check_error)?;
    let info = found.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current: u.current_version.clone(),
        notes: u.body.clone(),
    });
    *app.state::<Updates>().0.lock().unwrap() = found;
    let label = match &info {
        Some(i) => {
            tracing::info!("update available: {}", i.version);
            let _ = app.emit("update", i);
            format!("Install update {}…", i.version)
        }
        None => "Check for updates…".to_string(),
    };
    let _ = app.state::<UpdateMenuItem>().0.set_text(label);
    Ok(info)
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Bring the window up for a pairing code. Windows and macOS may refuse to
/// focus a background app, so also ask for attention and keep the window on
/// top while the code is showing.
fn show_for_pairing(app: &AppHandle) {
    show_main(app);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_always_on_top(true);
        let _ = w.request_user_attention(Some(UserAttentionType::Critical));
    }
}

/// Backends that do nothing, used when the platform could not be set up so
/// the window can still explain what went wrong.
fn fallback_backends(reason: String) -> Backends {
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    Backends {
        os: OsKind::current(),
        capture: Box::new(NullCapture { displays: vec![], status: BackendStatus::Error(reason.clone()) }),
        capture_events: rx,
        emulation: Box::new(NullEmulation { status: BackendStatus::Error(reason) }),
        screen: Box::new(Vec::new),
        clipboard: Box::new(NullClipboard),
    }
}

fn tray_text(s: &Snapshot) -> (String, &'static str) {
    let name = |id: &str| s.peers.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
    let tip = match &s.focus {
        _ if !s.settings.enabled => "Skerry: paused".to_string(),
        FocusView::Local => {
            let n = s.peers.iter().filter(|p| p.online).count();
            format!("Skerry: {n} computer{} connected", if n == 1 { "" } else { "s" })
        }
        FocusView::Controlling(p) => format!("Skerry: controlling {}", name(p)),
        FocusView::ControlledBy(p) => format!("Skerry: controlled by {}", name(p)),
    };
    (tip, if s.settings.enabled { "Pause sharing" } else { "Resume sharing" })
}

/// Log to the terminal and to daily files in `<config dir>/logs` (the last
/// week is kept), so problems can be diagnosed after the fact.
fn init_logging(log_dir: &std::path::Path) {
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tao=warn,wry=warn".into());
    let file = std::fs::create_dir_all(log_dir).ok().and_then(|_| {
        tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("skerry")
            .filename_suffix("log")
            .max_log_files(7)
            .build(log_dir)
            .ok()
    });
    let file_layer = file.map(|f| tracing_subscriber::fmt::layer().with_ansi(false).with_writer(f));
    tracing_subscriber::registry().with(filter).with(tracing_subscriber::fmt::layer()).with(file_layer).init();
}

fn main() {
    skerry_platform::init_process();
    let paths = match Paths::default_location() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Skerry: {e:#}");
            std::process::exit(1);
        }
    };
    let log_dir = paths.dir.join("logs");
    init_logging(&log_dir);
    tracing::info!(
        "Skerry {} starting on {} ({})",
        skerry_core::APP_VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    let minimized = std::env::args().any(|a| a == "--minimized");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Updates::default())
        .setup(move |app| {
            let config_path = paths.config.display().to_string();
            let (engine, backend) = tauri::async_runtime::block_on(async move {
                let (backends, backend) = match skerry_platform::backends().await {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::error!("input backends unavailable: {e:#}");
                        (fallback_backends(format!("{e:#}")), "unavailable".to_string())
                    }
                };
                let engine = engine::start(EngineOptions::new(paths), backends).await?;
                anyhow::Ok((engine, backend))
            })?;
            tracing::info!("Skerry started ({backend})");

            // Tray icon.
            let open = MenuItem::with_id(app, "open", "Open Skerry", true, None::<&str>)?;
            let toggle = MenuItem::with_id(app, "toggle", "Pause sharing", true, None::<&str>)?;
            let scan = MenuItem::with_id(app, "scan", "Scan network", true, None::<&str>)?;
            let updates = MenuItem::with_id(app, "updates", "Check for updates…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Skerry", true, None::<&str>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&open, &toggle, &scan, &sep1, &updates, &sep2, &quit])?;
            app.manage(UpdateMenuItem(updates.clone()));
            let mut tray = TrayIconBuilder::with_id("skerry")
                .menu(&menu)
                .tooltip("Skerry")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id().as_ref() {
                    "open" => show_main(app),
                    "toggle" => {
                        let st = app.state::<AppState>();
                        let enabled = st.engine.snapshot().settings.enabled;
                        st.engine.update_settings(SettingsUpdate { enabled: Some(!enabled), ..Default::default() });
                    }
                    "scan" => {
                        app.state::<AppState>().engine.rescan();
                        show_main(app);
                    }
                    "updates" => {
                        // The window shows the result and the install button.
                        show_main(app);
                        let _ = app.emit("menu-check-update", ());
                    }
                    "quit" => {
                        let engine = app.state::<AppState>().engine.clone();
                        tauri::async_runtime::block_on(engine.shutdown());
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left, button_state: MouseButtonState::Up, ..
                    } = ev
                    {
                        show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            // Forward engine events to the window and keep the tray current.
            let handle = app.handle().clone();
            let mut events = engine.subscribe();
            let toggle_item: MenuItem<Wry> = toggle.clone();
            tauri::async_runtime::spawn(async move {
                let mut announced = std::collections::HashSet::new();
                let mut on_top = false;
                loop {
                    match events.recv().await {
                        Ok(ev) => {
                            if let EngineEvent::State(s) = &ev {
                                let (tip, label) = tray_text(s);
                                if let Some(t) = handle.tray_by_id("skerry") {
                                    let _ = t.set_tooltip(Some(&tip));
                                }
                                let _ = toggle_item.set_text(label);
                                // Someone wants to pair: bring the window up so the code is visible.
                                let mut showing_code = false;
                                for p in &s.pairings {
                                    if p.stage == "show_code" {
                                        showing_code = true;
                                        if announced.insert(p.session) {
                                            show_for_pairing(&handle);
                                            on_top = true;
                                        }
                                    }
                                }
                                if on_top && !showing_code {
                                    on_top = false;
                                    if let Some(w) = handle.get_webview_window("main") {
                                        let _ = w.set_always_on_top(false);
                                    }
                                }
                            }
                            let _ = handle.emit("engine", &ev);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });

            // Look for updates now and then, unless turned off in settings.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(FIRST_UPDATE_CHECK).await;
                loop {
                    let enabled = handle.state::<AppState>().engine.snapshot().settings.check_updates;
                    if enabled {
                        // Failures are logged by check_error.
                        let _ = find_update(&handle).await;
                    }
                    tokio::time::sleep(UPDATE_CHECK_INTERVAL).await;
                }
            });

            #[cfg(target_os = "macos")]
            login_item::upgrade(&app.config().identifier);

            app.manage(AppState { engine, backend, config_path, log_dir });
            if !minimized {
                show_main(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps Skerry running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            get_info,
            pair,
            submit_code,
            cancel_pairing,
            set_layout,
            forget,
            update_settings,
            set_speed,
            add_manual_peer,
            remove_manual_peer,
            rescan,
            get_autostart,
            set_autostart,
            open_permission_settings,
            reset_permissions,
            get_diagnostics,
            open_logs,
            firewall_status,
            fix_firewall,
            check_update,
            install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Skerry");

    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = event {
            show_main(app);
        }
        if let RunEvent::ExitRequested { api, code, .. } = &event {
            // Only quit from the tray menu; closing the last window must not.
            if code.is_none() {
                api.prevent_exit();
            }
        }
        let _ = app;
    });
}
