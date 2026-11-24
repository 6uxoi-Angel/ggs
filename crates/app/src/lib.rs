//! Integra application shell: Tauri commands, events, tray, autostart,
//! benchmark, maintenance orchestration and the bridge glue.

mod benchmark;
mod bridge_store;
mod dns;
mod netcheck;
mod tgproxy;
mod win;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Manager;

use integra_bridge::BridgeServer;
use integra_core::config::AppConfig;
use integra_core::i18n::tr_with;
use integra_core::{
    ConfigStore, ListsManager, LogBuffer, LogEntry, LogLevel, ThemeColors, ThemePreset,
};
use integra_netmon::{Netmon, ProcessFilter, TrackerConfig};
use integra_zapret::{ManagerEvent, Status, ZapretError, ZapretManager};

use base64::Engine as _;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Serializable command error: an i18n key plus a ready localized message.
#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub key: String,
    pub message: String,
}

impl CommandError {
    pub fn key(key: &str) -> Self {
        Self {
            key: key.into(),
            message: integra_core::i18n::tr(key),
        }
    }
    pub fn key_with(key: &str, args: &[(&str, &str)]) -> Self {
        Self {
            key: key.into(),
            message: tr_with(key, args),
        }
    }
    pub fn raw(message: impl Into<String>) -> Self {
        Self {
            key: "common.error".into(),
            message: message.into(),
        }
    }
}

impl From<ZapretError> for CommandError {
    fn from(err: ZapretError) -> Self {
        let key = err.i18n_key();
        let message = match &err {
            ZapretError::LauncherExited { code, output } => tr_with(
                "zapret.exit_code",
                &[("code", &code.to_string()), ("suffix", output.as_str())],
            ),
            ZapretError::Timeout(suffix) => tr_with("zapret.timeout", &[("suffix", suffix)]),
            ZapretError::StartFailed(e) => tr_with("zapret.start_failed", &[("error", e)]),
            ZapretError::StopPartial(e) => tr_with("zapret.stop_partial", &[("errors", e)]),
            ZapretError::InstallFailed(e) => tr_with("zapret.install_failed", &[("error", e)]),
            ZapretError::RemoveFailed(e) => tr_with("zapret.remove_failed", &[("error", e)]),
            ZapretError::StrategyMissing(name) => {
                tr_with("zapret.strategy_missing", &[("strategy", name)])
            }
            ZapretError::Maintenance(e) => tr_with("maint.failed", &[("error", e)]),
            other => integra_core::i18n::tr(other.i18n_key()),
        };
        Self {
            key: key.into(),
            message,
        }
    }
}

impl From<std::io::Error> for CommandError {
    fn from(err: std::io::Error) -> Self {
        CommandError::raw(err.to_string())
    }
}

type CmdResult<T> = Result<T, CommandError>;

#[derive(Clone, Serialize)]
struct ProfileView {
    #[serde(flatten)]
    profile: integra_core::ZapretProfile,
    health: integra_core::ProfileHealth,
    status: Status,
    pid: u32,
    service_installed: bool,
    service_running: bool,
    strategies_count: usize,
}

/// Global application state shared with every command.
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub logs: Arc<LogBuffer>,
    pub lists: Arc<Mutex<ListsManager>>,
    pub managers: bridge_store::ManagersMap,
    pub managers_tx: mpsc::Sender<ManagerEvent>,
    pub netmon: Arc<Netmon>,
    pub bridge: Arc<BridgeServer>,
    pub benchmark: Mutex<Option<Arc<benchmark::Benchmark>>>,
    pub tgproxy: Arc<Mutex<tgproxy::TgProxyInner>>,
}

// parking_lot::Mutex is not `Clone`, so clone every inner value explicitly.
impl Clone for AppState {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            logs: self.logs.clone(),
            lists: self.lists.clone(),
            managers: self.managers.clone(),
            managers_tx: self.managers_tx.clone(),
            netmon: self.netmon.clone(),
            bridge: self.bridge.clone(),
            benchmark: Mutex::new(self.benchmark.lock().clone()),
            tgproxy: self.tgproxy.clone(),
        }
    }
}

impl AppState {
    fn ensure_manager(&self, profile_id: &str) -> Result<Arc<ZapretManager>, CommandError> {
        let profile = self
            .config
            .get()
            .profiles
            .into_iter()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| CommandError::key("zapret.path_missing"))?;
        let mut managers = self.managers.lock();
        if let Some(existing) = managers.get(profile_id) {
            existing.set_path(&profile.path);
            return Ok(existing.clone());
        }
        let manager = Arc::new(ZapretManager::new(profile_id, &profile.path));
        manager.attach_events(self.managers_tx.clone());
        managers.insert(profile_id.to_string(), manager.clone());
        Ok(manager)
    }

    fn manager_for(&self, profile_id: &str) -> Option<Arc<ZapretManager>> {
        self.ensure_manager(profile_id).ok()
    }

    fn profile(&self, profile_id: &str) -> Result<integra_core::ZapretProfile, CommandError> {
        self.config
            .get()
            .profiles
            .into_iter()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| CommandError::key("zapret.path_missing"))
    }

    fn log_debug(&self, source: &str, message: impl Into<String>) {
        self.logs.log(LogLevel::Debug, source, message);
    }

    fn log(&self, level: LogLevel, source: &str, message: impl Into<String>) {
        self.logs.log(level, source, message);
    }
}

fn min_level_from_config(config: &AppConfig) -> LogLevel {
    match config.log_level.to_ascii_uppercase().as_str() {
        "ALL" | "DEBUG" => LogLevel::Debug,
        "INFO" => LogLevel::Info,
        "SUCCESS" => LogLevel::Success,
        "WARN" => LogLevel::Warn,
        "ERROR" => LogLevel::Error,
        _ => LogLevel::Debug,
    }
}

/// Persisted log file for the developer console (`dev.log`).
fn start_dev_log_writer(logs: Arc<LogBuffer>) {
    let path = integra_core::paths::logs_dir().join("dev.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Rotate a huge file from the previous session.
    if std::fs::metadata(&path)
        .map(|m| m.len() > 8 * 1024 * 1024)
        .unwrap_or(false)
    {
        let _ = std::fs::rename(&path, path.with_extension("prev.log"));
    }
    logs.subscribe(Arc::new(move |entry: &LogEntry| {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(
                file,
                "{} [{:^7}] [{}] {}",
                entry.ts.format("%Y-%m-%d %H:%M:%S%.3f"),
                entry.level.as_str(),
                entry.source,
                entry.message
            );
        }
    }));
}

fn dev_log_path() -> PathBuf {
    integra_core::paths::logs_dir().join("dev.log")
}

// ── generic / settings ─────────────────────────────────────────────

#[tauri::command]
async fn i18n_bundle() -> Result<HashMap<&'static str, String>, CommandError> {
    Ok(integra_core::i18n::bundle()
        .into_iter()
        .map(|(k, v)| (k, v.to_string()))
        .collect())
}

#[tauri::command]
async fn get_config(state: tauri::State<'_, AppState>) -> Result<AppConfig, CommandError> {
    Ok(state.config.get())
}

#[tauri::command]
async fn set_language(
    state: tauri::State<'_, AppState>,
    language: String,
) -> Result<(), CommandError> {
    integra_core::i18n::set_language(&language);
    state
        .config
        .update(|c| c.language = language.clone())
        .map_err(|e| CommandError::raw(e.to_string()))?;
    state.log_debug("app::settings", format!("language → {language}"));
    Ok(())
}

#[tauri::command]
async fn set_theme(state: tauri::State<'_, AppState>, theme: String) -> Result<(), CommandError> {
    if !integra_core::config::is_valid_theme(&theme) {
        return Err(CommandError::key("theme.preset_applied"));
    }
    state
        .config
        .update(|c| c.theme = theme.clone())
        .map_err(|e| CommandError::raw(e.to_string()))?;
    state.log_debug("app::settings", format!("theme → {theme}"));
    Ok(())
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct SettingsPatch {
    start_minimized: Option<bool>,
    close_to_tray: Option<bool>,
    autostart_windows: Option<bool>,
    autostart_minimized: Option<bool>,
    log_level: Option<String>,
    bridge_enabled: Option<bool>,
    bridge_port: Option<u16>,
    netmon_poll_ms: Option<u64>,
    netmon_resolve_hosts: Option<bool>,
    netmon_auto_new: Option<bool>,
    netmon_max_entries: Option<usize>,
    auto_check_zapret: Option<bool>,
    auto_install_zapret: Option<bool>,
    auto_update_zapret: Option<bool>,
    taskbar: Option<integra_core::TaskbarConfig>,
}

#[tauri::command]
async fn update_settings(
    state: tauri::State<'_, AppState>,
    patch: SettingsPatch,
) -> Result<AppConfig, CommandError> {
    let restart_bridge = {
        let mut restart = false;
        state.config.update(|c| {
            if let Some(v) = patch.start_minimized {
                c.start_minimized = v;
            }
            if let Some(v) = patch.close_to_tray {
                c.close_to_tray = v;
            }
            if let Some(v) = patch.autostart_windows {
                c.autostart_windows = v;
            }
            if let Some(v) = patch.autostart_minimized {
                c.autostart_minimized = v;
            }
            if let Some(v) = &patch.log_level {
                c.log_level = v.clone();
                state.logs.set_min_level(min_level_from_config(c));
            }
            if let Some(v) = patch.bridge_enabled {
                c.bridge.enabled = v;
            }
            if let Some(v) = patch.bridge_port {
                if v != c.bridge.port {
                    c.bridge.port = v;
                    restart = state.bridge.is_running();
                }
            }
            if let Some(v) = patch.netmon_poll_ms {
                c.netmon.poll_interval_ms = v;
            }
            if let Some(v) = patch.netmon_resolve_hosts {
                c.netmon.resolve_hosts = v;
            }
            if let Some(v) = patch.netmon_auto_new {
                c.netmon.auto_include_new_processes = v;
            }
            if let Some(v) = patch.netmon_max_entries {
                c.netmon.max_entries = v;
            }
            if let Some(v) = patch.auto_check_zapret {
                c.auto_check_zapret = v;
            }
            if let Some(v) = patch.auto_install_zapret {
                c.auto_install_zapret = v;
            }
            if let Some(v) = patch.auto_update_zapret {
                c.auto_update_zapret = v;
            }
            if let Some(taskbar) = &patch.taskbar {
                c.taskbar = taskbar.clone();
            }
        })?;
        restart
    };
    state.log_debug("app::settings", "settings updated");
    if restart_bridge {
        state.bridge.stop();
        let port = state.bridge.start().map_err(CommandError::raw)?;
        state.log(
            LogLevel::Info,
            "bridge",
            tr_with("bridge.started", &[("port", &port.to_string())]),
        );
    }
    Ok(state.config.get())
}

// ── remote control (agent) ──────────────────────────────────────────

#[tauri::command]
async fn netcheck_run() -> Result<netcheck::NetcheckReport, CommandError> {
    tauri::async_runtime::spawn_blocking(netcheck::run)
        .await
        .map_err(|e| CommandError::raw(format!("netcheck: {e}")))
}

#[tauri::command]
async fn save_window_state(
    state: tauri::State<'_, AppState>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    maximized: bool,
) -> Result<(), CommandError> {
    // Ignore saves while the window is hidden or minimized (Windows then
    // reports -32000,-32000): a stale off-screen position must never be
    // persisted, or the next start would restore the window off-screen.
    if x <= -30000 || y <= -30000 {
        return Ok(());
    }
    state
        .config
        .update(|c| {
            c.window = integra_core::config::WindowConfig {
                width,
                height,
                x: Some(x),
                y: Some(y),
                maximized,
            };
        })
        .map_err(|e| CommandError::raw(e.to_string()))
}

#[tauri::command]
async fn app_info(state: tauri::State<'_, AppState>) -> Result<Value, CommandError> {
    Ok(json!({
        "version": APP_VERSION,
        "config_path": state.config.path().display().to_string(),
        "dev_log_path": dev_log_path().display().to_string(),
        "shared_lists_dir": bridge_store::shared_lists_root().display().to_string(),
        "is_admin": win::is_admin(),
        "os": std::env::consts::OS,
    }))
}

// ── first-run component check ──────────────────────────────────────

#[derive(Serialize)]
struct ComponentStatus {
    id: String,
    ok: bool,
    detail: Option<String>,
}

/// Is a WebView2 runtime installed (version folder under EdgeWebView)?
fn webview2_present() -> bool {
    for base in [
        r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application",
        r"C:\Program Files\Microsoft\EdgeWebView\Application",
    ] {
        let p = Path::new(base);
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(p) {
                if rd.filter_map(|e| e.ok()).any(|e| e.path().is_dir()) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check every component Integra needs; a missing item gets an action in UI.
#[tauri::command]
fn system_check(state: tauri::State<'_, AppState>) -> Result<Vec<ComponentStatus>, CommandError> {
    let mut out = Vec::new();
    out.push(ComponentStatus {
        id: "webview2".into(),
        ok: webview2_present(),
        detail: None,
    });
    out.push(ComponentStatus {
        id: "vcredist".into(),
        ok: Path::new(r"C:\Windows\System32\VCRUNTIME140.dll").exists(),
        detail: None,
    });
    let admin = win::is_admin();
    out.push(ComponentStatus {
        id: "admin".into(),
        ok: admin,
        detail: None,
    });
    let profiles = state.config.get().profiles;
    out.push(ComponentStatus {
        id: "zapret".into(),
        ok: !profiles.is_empty(),
        detail: profiles
            .first()
            .map(|p| p.path.to_string_lossy().into_owned()),
    });
    Ok(out)
}

/// Mark the first-run check as acknowledged (stored in config).
#[tauri::command]
fn complete_first_run(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    state
        .config
        .update(|c| c.first_run_done = true)
        .map_err(|e| CommandError::raw(e.to_string()))
}

#[tauri::command]
async fn relaunch_as_admin(app: tauri::AppHandle) -> Result<(), CommandError> {
    win::relaunch_as_admin().map_err(CommandError::raw)?;
    app.exit(0);
    Ok(())
}

// ── profiles ───────────────────────────────────────────────────────

fn profile_view(state: &AppState, profile: integra_core::ZapretProfile) -> ProfileView {
    let strategies = integra_zapret::strategy::discover(&profile.path);
    let (status, pid, service_installed, service_running) = match state.manager_for(&profile.id) {
        Some(manager) => {
            let (installed, running) = manager.service_state(false);
            (manager.status(), manager.pid(), installed, running)
        }
        None => (Status::Stopped, 0, false, false),
    };
    ProfileView {
        health: profile.inspect(),
        profile,
        status,
        pid,
        service_installed,
        service_running,
        strategies_count: strategies.len(),
    }
}

#[tauri::command]
async fn list_profiles(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProfileView>, CommandError> {
    Ok(state
        .config
        .get()
        .profiles
        .into_iter()
        .map(|p| profile_view(&state, p))
        .collect())
}

#[tauri::command]
async fn add_profile(
    state: tauri::State<'_, AppState>,
    name: String,
    path: String,
) -> Result<ProfileView, CommandError> {
    let path = PathBuf::from(path.trim());
    if path.as_os_str().is_empty() {
        return Err(CommandError::key("profile.health_missing_path"));
    }
    let mut profile = integra_core::ZapretProfile::new(name.trim(), &path);
    if profile.name.is_empty() {
        profile.name = "Zapret".into();
    }
    let mut view = None;
    state.config.update(|c| {
        if c.profiles.iter().any(|p| p.path == profile.path) {
            return; // duplicate
        }
        c.profiles.push(profile.clone());
        if c.active_profile.is_none() {
            c.active_profile = Some(profile.id.clone());
        }
        view = Some(profile.clone());
    })?;
    let Some(profile) = view else {
        return Err(CommandError::key("profile.duplicate_path"));
    };
    state.log(
        LogLevel::Success,
        "app::profiles",
        format!("profile added: {}", profile.name),
    );
    Ok(profile_view(&state, profile))
}

#[tauri::command]
async fn update_profile(
    state: tauri::State<'_, AppState>,
    profile: integra_core::ZapretProfile,
) -> Result<ProfileView, CommandError> {
    let mut updated = None;
    state.config.update(|c| {
        if let Some(slot) = c.profiles.iter_mut().find(|p| p.id == profile.id) {
            *slot = profile.clone();
            updated = Some(profile.clone());
        }
    })?;
    let profile = updated.ok_or_else(|| CommandError::key("zapret.path_missing"))?;
    state.log_debug(
        "app::profiles",
        format!("profile updated: {}", profile.name),
    );
    Ok(profile_view(&state, profile))
}

#[tauri::command]
async fn remove_profile(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<(), CommandError> {
    let mut removed = None;
    state.config.update(|c| {
        if let Some(index) = c.profiles.iter().position(|p| p.id == profile_id) {
            removed = Some(c.profiles.remove(index));
        }
        if c.active_profile.as_deref() == Some(profile_id.as_str()) {
            c.active_profile = c.profiles.first().map(|p| p.id.clone());
        }
    })?;
    if let Some(manager) = state.managers.lock().remove(&profile_id) {
        let _ = manager.stop();
    }
    let name = removed.map(|p| p.name).unwrap_or_default();
    state.log(
        LogLevel::Info,
        "app::profiles",
        format!("profile removed: {name}"),
    );
    Ok(())
}

#[tauri::command]
async fn set_active_profile(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<(), CommandError> {
    state.profile(&profile_id)?;
    state
        .config
        .update(|c| c.active_profile = Some(profile_id.clone()))?;
    state.log_debug("app::profiles", format!("active profile → {profile_id}"));
    Ok(())
}

#[tauri::command]
async fn list_strategies(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<Vec<integra_zapret::Strategy>, CommandError> {
    let profile = state.profile(&profile_id)?;
    Ok(integra_zapret::strategy::discover(&profile.path))
}

#[tauri::command]
async fn start_profile(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    strategy: Option<String>,
) -> Result<u32, CommandError> {
    let profile = state.profile(&profile_id)?;
    let manager = state.ensure_manager(&profile_id)?;
    let strategy_file = strategy.unwrap_or_else(|| profile.selected_strategy.clone());
    let pid = manager.start(&strategy_file).map_err(CommandError::from)?;
    state
        .config
        .update(|c| {
            if let Some(slot) = c.profiles.iter_mut().find(|p| p.id == profile_id) {
                slot.selected_strategy = strategy_file.clone();
            }
        })
        .map_err(|e| CommandError::raw(e.to_string()))?;
    Ok(pid)
}

#[tauri::command]
async fn stop_profile(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<u32, CommandError> {
    let manager = state.ensure_manager(&profile_id)?;
    manager.stop().map_err(CommandError::from)
}

#[tauri::command]
async fn profile_status(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<ProfileView, CommandError> {
    let profile = state.profile(&profile_id)?;
    Ok(profile_view(&state, profile))
}

#[tauri::command]
async fn install_service(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    strategy: String,
) -> Result<(), CommandError> {
    let manager = state.ensure_manager(&profile_id)?;
    manager
        .install_service(&strategy)
        .map_err(CommandError::from)
}

#[tauri::command]
async fn remove_service(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<(), CommandError> {
    let manager = state.ensure_manager(&profile_id)?;
    manager.remove_service().map_err(CommandError::from)
}

#[tauri::command]
async fn open_in_explorer(path: String) -> Result<(), CommandError> {
    win::open_in_explorer(Path::new(&path)).map_err(CommandError::raw)
}

#[tauri::command]
async fn reveal_config(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    win::reveal_file(state.config.path()).map_err(CommandError::raw)
}

// ── lists ──────────────────────────────────────────────────────────

fn with_list_root<T>(
    state: &AppState,
    profile_id: &str,
    f: impl FnOnce(&ListsManager) -> T,
) -> CmdResult<T> {
    let root =
        bridge_store::lists_root_for(&state.config.get(), profile_id).map_err(CommandError::raw)?;
    let lists = state.lists.lock();
    lists.set_root(root);
    Ok(f(&lists))
}

#[derive(Serialize)]
struct ListContent {
    id: String,
    entries: Vec<String>,
    count: usize,
}

#[tauri::command]
async fn list_read(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    list_id: String,
) -> Result<ListContent, CommandError> {
    if bridge_store::list_spec(&list_id).is_none() {
        return Err(CommandError::key("bridge.route_missing"));
    }
    with_list_root(&state, &profile_id, |lists| ListContent {
        id: list_id.clone(),
        entries: lists.read(&list_id),
        count: lists.count(&list_id),
    })
}

#[tauri::command]
async fn list_meta(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<Value, CommandError> {
    let config = state.config.get();
    let profile = config
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| CommandError::key("zapret.path_missing"))?;
    let mut counts = serde_json::Map::new();
    with_list_root(&state, &profile_id, |lists| {
        for spec in integra_core::LISTS {
            counts.insert(spec.id.to_string(), json!(lists.count(spec.id)));
        }
    })?;
    Ok(json!({
        "use_shared": profile.use_shared_lists,
        "counts": counts,
    }))
}

#[tauri::command]
async fn list_add(
    state: tauri::State<'_, AppState>,
    profile_id: Option<String>,
    list_id: String,
    entries: Vec<String>,
) -> Result<integra_core::lists::BulkReport, CommandError> {
    let profile_id = match profile_id {
        Some(id) => id,
        None => state
            .config
            .get()
            .active_profile
            .ok_or_else(|| CommandError::key("lists.no_active_profile"))?,
    };
    let report = with_list_root(&state, &profile_id, |lists| {
        lists
            .add(&list_id, &entries)
            .map_err(|e| CommandError::raw(e.to_string()))
    })??;
    state.log_debug(
        "app::lists",
        format!("list {list_id}: +{} entries", report.added.len()),
    );
    Ok(report)
}

#[tauri::command]
async fn list_remove(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    list_id: String,
    entry: String,
) -> Result<bool, CommandError> {
    with_list_root(&state, &profile_id, |lists| {
        lists
            .remove(&list_id, &entry)
            .map_err(|e| CommandError::raw(e.to_string()))
    })?
}

#[tauri::command]
async fn list_replace(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    list_id: String,
    lines: Vec<String>,
) -> Result<integra_core::lists::BulkReport, CommandError> {
    with_list_root(&state, &profile_id, |lists| {
        lists
            .replace(&list_id, &lines)
            .map_err(|e| CommandError::raw(e.to_string()))
    })?
}

#[tauri::command]
async fn list_remove_from_all(
    state: tauri::State<'_, AppState>,
    profile_id: String,
    entry: String,
) -> Result<Vec<String>, CommandError> {
    with_list_root(&state, &profile_id, |lists| {
        lists
            .remove_from_all(&entry)
            .map_err(|e| CommandError::raw(e.to_string()))
    })?
}

// ── netmon ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(tag = "mode", content = "value", rename_all = "snake_case")]
enum FilterDto {
    All,
    Name(String),
    Pids(Vec<u32>),
}

impl FilterDto {
    fn into_filter(self) -> ProcessFilter {
        match self {
            FilterDto::All => ProcessFilter::All,
            FilterDto::Name(name) => ProcessFilter::Name(name),
            FilterDto::Pids(pids) => ProcessFilter::Pids(pids),
        }
    }
}

#[tauri::command]
async fn netmon_start(
    state: tauri::State<'_, AppState>,
    filter: FilterDto,
) -> Result<(), CommandError> {
    let config = state.config.get();
    let tracker = TrackerConfig {
        filter: filter.into_filter(),
        poll_interval: Duration::from_millis(config.netmon.poll_interval_ms.clamp(200, 30_000)),
        resolve_hosts: config.netmon.resolve_hosts,
        max_entries: config.netmon.max_entries.clamp(100, 100_000),
        process_refresh: Duration::from_secs(3),
    };
    state
        .netmon
        .start(tracker)
        .map_err(|e| CommandError::key(e.to_string().as_str()))?;
    state.log(LogLevel::Success, "netmon", "tracking started");
    Ok(())
}

#[tauri::command]
async fn netmon_stop(
    state: tauri::State<'_, AppState>,
) -> Result<integra_netmon::SessionSummary, CommandError> {
    let summary = state
        .netmon
        .stop()
        .map_err(|e| CommandError::key(e.to_string().as_str()))?;
    state.log(
        LogLevel::Info,
        "netmon",
        tr_with(
            "netmon.session_stopped",
            &[("count", &summary.entries.to_string())],
        ),
    );
    Ok(summary)
}

#[tauri::command]
async fn netmon_status(state: tauri::State<'_, AppState>) -> Result<Value, CommandError> {
    Ok(json!({
        "running": state.netmon.is_running(),
        "summary": state.netmon.last_summary(),
    }))
}

#[tauri::command]
async fn netmon_entries(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<integra_netmon::TrackedEntry>, CommandError> {
    Ok(state.netmon.entries(limit.unwrap_or(500) as usize))
}

/// Aggregated "who is consuming traffic" view.
#[derive(Serialize)]
struct ProcessTraffic {
    pid: u32,
    process: String,
    connections: usize,
    hits: u64,
    active: bool,
    local_ips: Vec<String>,
    top_remotes: Vec<String>,
}

#[tauri::command]
async fn netmon_summary(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProcessTraffic>, CommandError> {
    let entries = state.netmon.entries(usize::MAX);
    let now = chrono::Local::now();
    let mut order: Vec<(u32, String)> = Vec::new();
    let mut groups: HashMap<(u32, String), Vec<integra_netmon::TrackedEntry>> = HashMap::new();
    for entry in entries {
        let key = (entry.pid, entry.process.clone());
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(entry);
    }
    let mut out: Vec<ProcessTraffic> = order
        .into_iter()
        .filter_map(|key| {
            let rows = groups.remove(&key)?;
            let mut hits = 0u64;
            let mut active = false;
            let mut ips: Vec<String> = Vec::new();
            let mut remotes: HashMap<String, u64> = HashMap::new();
            for row in &rows {
                hits += row.hits as u64;
                if (now - row.last_seen).num_milliseconds() < 4000 {
                    active = true;
                }
                if let Some((ip, _)) = row.local.rsplit_once(':') {
                    let ip = ip.trim_matches(['[', ']']).to_string();
                    if !ips.contains(&ip) {
                        ips.push(ip);
                    }
                }
                let label = row
                    .host
                    .clone()
                    .or(row.remote.clone())
                    .unwrap_or_else(|| row.local.clone());
                *remotes.entry(label).or_default() += 1;
            }
            let mut top: Vec<(String, u64)> = remotes.into_iter().collect();
            top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            Some(ProcessTraffic {
                pid: key.0,
                process: key.1,
                connections: rows.len(),
                hits,
                active,
                local_ips: ips.into_iter().take(3).collect(),
                top_remotes: top.into_iter().take(4).map(|(label, _)| label).collect(),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then(a.process.cmp(&b.process))
    });
    Ok(out)
}

#[tauri::command]
async fn netmon_processes(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<integra_netmon::ProcessInfo>, CommandError> {
    Ok(state.netmon.processes())
}

#[tauri::command]
async fn netmon_export_csv(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<(), CommandError> {
    let csv = state.netmon.export_csv();
    std::fs::write(&path, csv).map_err(|e| CommandError::raw(e.to_string()))?;
    state.log(LogLevel::Success, "netmon", format!("CSV → {path}"));
    Ok(())
}

// ── diagnostics ────────────────────────────────────────────────────

fn all_diag_sites(state: &AppState) -> Vec<integra_diag::Site> {
    let config = state.config.get();
    let mut sites = integra_diag::default_sites();
    for custom in &config.diagnostics.custom_sites {
        sites.push(integra_diag::Site {
            id: custom.id.clone(),
            name: custom.name.clone(),
            host: custom.host.clone(),
            port: custom.port,
        });
    }
    sites
}

#[tauri::command]
async fn diag_sites(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<integra_diag::Site>, CommandError> {
    Ok(all_diag_sites(&state))
}

#[tauri::command]
async fn diag_add_site(
    state: tauri::State<'_, AppState>,
    name: String,
    host: String,
    port: u16,
) -> Result<Vec<integra_diag::Site>, CommandError> {
    let normalized = integra_core::normalize_host(&host, true).map_err(|e| CommandError {
        key: e.i18n_key().into(),
        message: e.to_string(),
    })?;
    let site = integra_core::config::CustomSite {
        id: integra_core::random_hex(4),
        name: if name.trim().is_empty() {
            normalized.clone()
        } else {
            name.trim().to_string()
        },
        host: normalized,
        port: if port == 0 { 443 } else { port },
    };
    state.config.update(|c| {
        c.diagnostics.custom_sites.push(site);
        c.diagnostics
            .custom_sites
            .truncate(integra_core::consts::MAX_CUSTOM_SITES);
    })?;
    Ok(all_diag_sites(&state))
}

#[tauri::command]
async fn diag_remove_site(
    state: tauri::State<'_, AppState>,
    site_id: String,
) -> Result<Vec<integra_diag::Site>, CommandError> {
    state.config.update(|c| {
        c.diagnostics.custom_sites.retain(|s| s.id != site_id);
    })?;
    Ok(all_diag_sites(&state))
}

#[derive(Serialize)]
struct SiteReport {
    site: integra_diag::Site,
    outcomes: Vec<integra_diag::CheckOutcome>,
}

#[tauri::command]
async fn diag_run(
    state: tauri::State<'_, AppState>,
    kinds: Vec<String>,
    rounds: u32,
    timeout_ms: u64,
) -> Result<Vec<SiteReport>, CommandError> {
    let sites = all_diag_sites(&state);
    let kinds: Vec<integra_diag::CheckKind> = kinds
        .iter()
        .filter_map(|k| match k.as_str() {
            "dns" => Some(integra_diag::CheckKind::Dns),
            "tcp" => Some(integra_diag::CheckKind::Tcp),
            "tls" => Some(integra_diag::CheckKind::Tls),
            "http" => Some(integra_diag::CheckKind::Http),
            _ => None,
        })
        .collect();
    if kinds.is_empty() {
        return Ok(Vec::new());
    }
    let rounds = rounds.clamp(1, 10);
    let timeout = Duration::from_millis(timeout_ms.clamp(500, 15_000));
    state.log_debug(
        "diag",
        format!(
            "running checks: {} sites, {} kinds",
            sites.len(),
            kinds.len()
        ),
    );
    let reports: Vec<SiteReport> = sites
        .iter()
        .map(|site| SiteReport {
            site: site.clone(),
            outcomes: integra_diag::check_site(site, &kinds, rounds, timeout),
        })
        .collect();
    Ok(reports)
}

// ── benchmark ──────────────────────────────────────────────────────

#[tauri::command]
async fn benchmark_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<(), CommandError> {
    {
        let existing = state.benchmark.lock();
        if let Some(bench) = existing.as_ref() {
            if bench.is_running() {
                return Err(CommandError::key("bench.running"));
            }
        }
    }
    let profile = state.profile(&profile_id)?;
    let manager = state.ensure_manager(&profile_id)?;
    let strategies: Vec<String> = integra_zapret::strategy::discover(&profile.path)
        .into_iter()
        .map(|s| s.file)
        .collect();
    if strategies.is_empty() {
        return Err(CommandError::key("bench.no_strategies"));
    }
    let (installed, _) = manager.service_state(true);
    if installed {
        return Err(CommandError::key("bench.conflict"));
    }
    let sites: Vec<(String, String, u16)> = all_diag_sites(&state)
        .into_iter()
        .take(benchmark::MAX_BENCH_SITES)
        .map(|s| (s.id, s.host, s.port))
        .collect();

    let logs = state.logs.clone();
    let handle = benchmark::spawn(manager, strategies, sites, move |progress| {
        use tauri::Emitter;
        let _ = app.emit("benchmark", progress.clone());
        logs.log(
            LogLevel::Debug,
            "benchmark",
            format!(
                "{:?} {} ({}/{})",
                progress.phase, progress.strategy, progress.index, progress.total
            ),
        );
    });
    *state.benchmark.lock() = Some(handle);
    Ok(())
}

#[tauri::command]
async fn benchmark_status(state: tauri::State<'_, AppState>) -> Result<Value, CommandError> {
    let guard = state.benchmark.lock();
    match guard.as_ref() {
        Some(bench) => Ok(json!({
            "running": bench.is_running(),
            "progress": bench.progress(),
            "result": bench.result(),
        })),
        None => Ok(json!({"running": false, "progress": Value::Null, "result": Value::Null})),
    }
}

#[tauri::command]
async fn benchmark_cancel(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    if let Some(bench) = state.benchmark.lock().as_ref() {
        bench.cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

// ── bridge ─────────────────────────────────────────────────────────

#[tauri::command]
async fn bridge_status(state: tauri::State<'_, AppState>) -> Result<Value, CommandError> {
    Ok(json!({
        "running": state.bridge.is_running(),
        "port": state.bridge.actual_port(),
        "enabled": state.config.get().bridge.enabled,
    }))
}

#[tauri::command]
async fn bridge_start(state: tauri::State<'_, AppState>) -> Result<u16, CommandError> {
    let port = state.bridge.start().map_err(CommandError::raw)?;
    Ok(port)
}

#[tauri::command]
async fn bridge_stop(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    state.bridge.stop();
    Ok(())
}

#[tauri::command]
async fn bridge_pairing(state: tauri::State<'_, AppState>) -> Result<Value, CommandError> {
    Ok(json!({
        "code": state.bridge.pairing_code(),
        "expires_in": state.bridge.pairing_expires_in(),
    }))
}

#[tauri::command]
async fn bridge_regenerate_code(state: tauri::State<'_, AppState>) -> Result<Value, CommandError> {
    let code = state.bridge.regenerate_pairing();
    Ok(json!({
        "code": code,
        "expires_in": state.bridge.pairing_expires_in(),
    }))
}

// ── logs ───────────────────────────────────────────────────────────

#[tauri::command]
async fn logs_snapshot(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
    level: Option<String>,
) -> Result<Vec<LogEntry>, CommandError> {
    let limit = limit.unwrap_or(300).clamp(10, 5000) as usize;
    let entries = state.logs.snapshot(limit);
    let filtered = match level.as_deref() {
        None | Some("ALL") | Some("all") => entries,
        Some(want) => {
            let want = LogLevel::parse(want);
            entries.into_iter().filter(|e| e.level == want).collect()
        }
    };
    Ok(filtered)
}

#[tauri::command]
async fn logs_clear(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    state.logs.clear();
    Ok(())
}

#[tauri::command]
async fn logs_export(state: tauri::State<'_, AppState>, path: String) -> Result<(), CommandError> {
    let entries = state.logs.snapshot(usize::MAX);
    let mut body = String::new();
    for entry in entries {
        body.push_str(&format!(
            "{} [{:^7}] [{}] {}\n",
            entry.ts.format("%Y-%m-%d %H:%M:%S"),
            entry.level.as_str(),
            entry.source,
            entry.message
        ));
    }
    std::fs::write(&path, body).map_err(|e| CommandError::raw(e.to_string()))?;
    state.log(LogLevel::Success, "logs", format!("exported → {path}"));
    Ok(())
}

// ── developer mode ─────────────────────────────────────────────────

#[tauri::command]
async fn dev_enable(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    state.config.update(|c| c.dev_mode = true)?;
    state.log(LogLevel::Warn, "app::dev", "DEVELOPER MODE ENABLED");
    Ok(())
}

#[tauri::command]
async fn dev_disable(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    state.config.update(|c| c.dev_mode = false)?;
    state.log(LogLevel::Warn, "app::dev", "developer mode disabled");
    Ok(())
}

#[tauri::command]
async fn open_dev_logs(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), CommandError> {
    let path = dev_log_path();
    if !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
            .map_err(|e| CommandError::raw(e.to_string()))?;
        std::fs::write(&path, "").map_err(|e| CommandError::raw(e.to_string()))?;
    }
    win::open_console_tail(&path).map_err(CommandError::raw)?;
    if let Some(window) = app.get_webview_window("main") {
        window.open_devtools();
    }
    state.log(
        LogLevel::Info,
        "app::dev",
        format!("dev console attached to {}", path.display()),
    );
    Ok(())
}

// ── themes ─────────────────────────────────────────────────────────

#[tauri::command]
async fn theme_presets(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ThemePreset>, CommandError> {
    Ok(state.config.get().theme_presets)
}

#[tauri::command]
async fn theme_save_preset(
    state: tauri::State<'_, AppState>,
    name: String,
    base: Option<String>,
    colors: ThemeColors,
    tabbar_image: Option<String>,
    content_image: Option<String>,
) -> Result<ThemePreset, CommandError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(CommandError::key("theme.name_required"));
    }
    let mut preset = ThemePreset {
        id: integra_core::random_hex(4),
        name,
        base: base.unwrap_or_else(|| "dark_accent".into()),
        colors,
        tabbar_image,
        content_image,
    };
    if !integra_core::BUILTIN_THEMES.contains(&preset.base.as_str()) {
        preset.base = "dark_accent".into();
    }
    preset.colors.sanitize();
    state.config.update(|c| {
        c.theme_presets.push(preset.clone());
        c.theme = format!("preset:{}", preset.id);
    })?;
    state.log(
        LogLevel::Success,
        "app::theme",
        format!("preset saved: {}", preset.name),
    );
    Ok(preset)
}

#[tauri::command]
async fn theme_delete_preset(
    state: tauri::State<'_, AppState>,
    preset_id: String,
) -> Result<(), CommandError> {
    let mut reset_theme = false;
    state.config.update(|c| {
        c.theme_presets.retain(|p| p.id != preset_id);
        if c.theme == format!("preset:{preset_id}") {
            c.theme = "dark_accent".into();
            reset_theme = true;
        }
    })?;
    let _ = reset_theme;
    state.log_debug("app::theme", format!("preset deleted: {preset_id}"));
    Ok(())
}

#[tauri::command]
async fn read_image_data_url(path: String) -> Result<String, CommandError> {
    let bytes = std::fs::read(&path).map_err(|e| CommandError::raw(e.to_string()))?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(CommandError::key("theme.image_too_large"));
    }
    let mime = match Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    };
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Persist a cropped/optimized background image (data URL) into the app data
/// dir and return the stored path (used as a preset background reference).
#[tauri::command]
async fn save_background_image(
    app: tauri::AppHandle,
    data_url: String,
) -> Result<String, CommandError> {
    let (_, b64) = data_url
        .split_once(',')
        .ok_or_else(|| CommandError::raw("bad data url"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| CommandError::raw(e.to_string()))?;
    if bytes.is_empty() || bytes.len() > 16 * 1024 * 1024 {
        return Err(CommandError::raw("bad image size"));
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| CommandError::raw(e.to_string()))?
        .join("bg");
    std::fs::create_dir_all(&dir).map_err(|e| CommandError::raw(e.to_string()))?;
    let name = format!("bg_{}.png", integra_core::random_hex(4));
    let path = dir.join(name);
    std::fs::write(&path, bytes).map_err(|e| CommandError::raw(e.to_string()))?;
    Ok(path.to_string_lossy().into_owned())
}

// ── zapret maintenance ─────────────────────────────────────────────

#[tauri::command]
async fn zapret_check_update() -> Result<integra_zapret::maintenance::LatestRelease, CommandError> {
    integra_zapret::maintenance::fetch_latest().map_err(CommandError::from)
}

#[tauri::command]
async fn zapret_installed_version(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<Option<String>, CommandError> {
    let profile = state.profile(&profile_id)?;
    Ok(integra_zapret::maintenance::installed_version(
        &profile.path,
    ))
}

#[tauri::command]
async fn zapret_install_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    profile_id: String,
    version: String,
    url: String,
) -> Result<(), CommandError> {
    let profile = state.profile(&profile_id)?;
    let manager = state.ensure_manager(&profile_id)?;
    if manager.status() != Status::Stopped {
        return Err(CommandError::key("maint.running_profile"));
    }
    let logs = state.logs.clone();
    std::thread::spawn(move || {
        use integra_zapret::maintenance as maint;
        let emit = |payload: Value| {
            let _ = tauri::Emitter::emit(&app, "zapret-update", payload);
        };
        let temp_dir =
            std::env::temp_dir().join(format!("integra-zapret-{}", integra_core::random_hex(4)));
        let zip_path = temp_dir.join(format!("zapret-{version}.zip"));
        let result = (|| -> Result<(), CommandError> {
            logs.log(
                LogLevel::Info,
                "maint",
                tr_with("maint.downloading", &[("percent", "0")]),
            );
            let last = std::cell::Cell::new(u64::MAX);
            maint::download(&url, &zip_path, &|copied, total| {
                let percent = copied
                    .checked_mul(100)
                    .and_then(|v| v.checked_div(total))
                    .map(|v| v.min(100))
                    .unwrap_or(0);
                // Throttle event spam: report only when the percent changes.
                if percent != last.get() {
                    last.set(percent);
                    emit(json!({"phase": "download", "percent": percent}));
                }
            })
            .map_err(CommandError::from)?;
            emit(json!({"phase": "extract"}));
            logs.log(
                LogLevel::Info,
                "maint",
                integra_core::i18n::tr("maint.extracting"),
            );
            maint::install_release(&zip_path, &profile.path, &version)
                .map_err(CommandError::from)?;
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&temp_dir);
        match result {
            Ok(()) => {
                emit(json!({"phase": "done", "version": version}));
                logs.log(
                    LogLevel::Success,
                    "maint",
                    tr_with("maint.updated", &[("version", &version)]),
                );
            }
            Err(err) => {
                emit(json!({"phase": "error", "message": err.message}));
                logs.log(LogLevel::Error, "maint", err.message);
            }
        }
    });
    Ok(())
}

// ── Roblox DNS manager ─────────────────────────────────────────────

#[tauri::command]
fn dns_servers() -> Vec<dns::DnsServer> {
    dns::DNS_SERVERS.to_vec()
}

#[tauri::command]
async fn dns_benchmark() -> Result<Vec<dns::BenchResult>, CommandError> {
    let handle = tauri::async_runtime::spawn_blocking(dns::bench_dns);
    handle.await.map_err(|e| CommandError::raw(e.to_string()))
}

#[tauri::command]
fn dns_status() -> Result<Value, CommandError> {
    dns::dns_status()
}

#[tauri::command]
fn dns_apply(primary: String, secondary: String, use_doh: bool) -> Result<(), CommandError> {
    dns::dns_apply(&primary, &secondary, use_doh)
}

#[tauri::command]
fn dns_reset() -> Result<(), CommandError> {
    dns::dns_reset()
}

#[tauri::command]
async fn roblox_find() -> Result<Vec<dns::HostsCandidate>, CommandError> {
    let handle = tauri::async_runtime::spawn_blocking(dns::roblox_find);
    handle.await.map_err(|e| CommandError::raw(e.to_string()))
}

#[tauri::command]
fn roblox_hosts_state() -> Result<bool, CommandError> {
    dns::roblox_hosts_state()
}

#[tauri::command]
fn roblox_hosts_apply(ip: String) -> Result<String, CommandError> {
    dns::roblox_hosts_apply(&ip)
}

#[tauri::command]
fn roblox_hosts_revert() -> Result<bool, CommandError> {
    dns::roblox_hosts_revert()
}

// ── TG WS Proxy ────────────────────────────────────────────────────

fn tgproxy_log_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let p = PathBuf::from(appdata).join("Integra/logs");
        let _ = std::fs::create_dir_all(&p);
        return p;
    }
    std::env::temp_dir()
}

#[tauri::command]
fn tgproxy_status(state: tauri::State<'_, AppState>) -> Result<Value, CommandError> {
    let cfg = state.config.get();
    let dir = tgproxy::find_dir(&cfg.tgproxy.path);
    let mut secret = cfg.tgproxy.secret.clone();
    if secret.len() != 32 {
        secret = integra_core::random_hex(16);
        let s = secret.clone();
        let _ = state.config.update(move |c| c.tgproxy.secret = s);
    }
    {
        let mut inner = state.tgproxy.lock();
        if inner.child.is_some() && !inner.is_running() {
            inner.child = None;
        }
    }
    let running = state.tgproxy.lock().is_running()
        || tgproxy::port_open(cfg.tgproxy.port);
    let version = dir
        .as_ref()
        .map(|d| tgproxy::read_version(d))
        .unwrap_or_default();
    let venv_ok = dir
        .as_ref()
        .map(|d| d.join(".venv/Scripts/python.exe").exists())
        .unwrap_or(false);
    let link = format!(
        "tg://proxy?server=127.0.0.1&port={}&secret={}",
        cfg.tgproxy.port, secret
    );
    Ok(json!({
        "dir": dir.as_ref().map(|d| d.to_string_lossy().into_owned()),
        "exists": dir.is_some(),
        "version": version,
        "venv_ok": venv_ok,
        "setup_running": state.tgproxy.lock().setup_running,
        "running": running,
        "port": cfg.tgproxy.port,
        "secret": secret,
        "link": link,
    }))
}

#[tauri::command]
fn tgproxy_set_path(state: tauri::State<'_, AppState>, path: String) -> Result<(), CommandError> {
    state
        .config
        .update(move |c| c.tgproxy.path = path.clone())
        .map_err(|e| CommandError::raw(e.to_string()))?;
    Ok(())
}

#[tauri::command]
async fn tgproxy_setup(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    let cfg = state.config.get();
    if state.tgproxy.lock().setup_running {
        return Err(CommandError::key("tgproxy.setup_running"));
    }
    let dir = tgproxy::find_dir(&cfg.tgproxy.path)
        .ok_or_else(|| CommandError::key("tgproxy.not_found"))?;
    let shared = state.tgproxy.clone();
    std::thread::spawn(move || {
        if let Err(e) = tgproxy::setup_venv(shared.clone(), &dir) {
            shared.lock().push_log(format!("[integra] {}", e.message));
        }
    });
    Ok(())
}

#[tauri::command]
fn tgproxy_start(
    state: tauri::State<'_, AppState>,
    port: Option<u16>,
    secret: Option<String>,
) -> Result<(), CommandError> {
    let mut cfg = state.config.get();
    let p = port.unwrap_or(cfg.tgproxy.port).clamp(1024, 65535);
    let s = match secret {
        Some(s) if s.len() == 32 => s,
        _ if cfg.tgproxy.secret.len() == 32 => cfg.tgproxy.secret.clone(),
        _ => {
            let g = integra_core::random_hex(16);
            cfg.tgproxy.secret = g.clone();
            g
        }
    };
    if cfg.tgproxy.port != p || cfg.tgproxy.secret != s {
        cfg.tgproxy.port = p;
        cfg.tgproxy.secret = s.clone();
        let saved = cfg.tgproxy.clone();
        let _ = state.config.update(move |c| c.tgproxy = saved);
    }
    let dir = tgproxy::find_dir(&cfg.tgproxy.path)
        .ok_or_else(|| CommandError::key("tgproxy.not_found"))?;
    let shared = state.tgproxy.clone();
    let log_dir = tgproxy_log_dir();
    tgproxy::spawn_proxy(shared, &dir, p, &s, &log_dir)?;
    Ok(())
}

#[tauri::command]
fn tgproxy_stop(state: tauri::State<'_, AppState>) -> Result<(), CommandError> {
    tgproxy::stop_proxy(state.tgproxy.clone());
    Ok(())
}

#[tauri::command]
fn tgproxy_logs(state: tauri::State<'_, AppState>, n: Option<usize>) -> Vec<String> {
    state.tgproxy.lock().tail(n.unwrap_or(200))
}

// ── app bootstrap ──────────────────────────────────────────────────

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn stop_all_profiles(state: &AppState) {
    for manager in state.managers.lock().values() {
        let _ = manager.stop();
    }
}

fn autostart_profiles(state: Arc<AppState>) {
    std::thread::Builder::new()
        .name("integra-autostart".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let config = state.config.get();
            for profile in config.profiles.iter().filter(|p| p.autostart && p.enabled) {
                let manager = match state.ensure_manager(&profile.id) {
                    Ok(manager) => manager,
                    Err(err) => {
                        state.log(LogLevel::Error, "zapret::autostart", err.message);
                        continue;
                    }
                };
                match manager.start(&profile.selected_strategy) {
                    Ok(pid) => state.log(
                        LogLevel::Success,
                        "zapret::autostart",
                        tr_with(
                            "zapret.started",
                            &[
                                ("strategy", &profile.selected_strategy),
                                ("pid", &pid.to_string()),
                            ],
                        ),
                    ),
                    Err(err) => state.log(
                        LogLevel::Error,
                        "zapret::autostart",
                        CommandError::from(err).message,
                    ),
                }
            }
        })
        .ok();
}

pub fn run() {
    let config = Arc::new(ConfigStore::load(None));
    integra_core::i18n::set_language(&config.get().language);
    let logs = LogBuffer::new(5000);
    logs.set_min_level(min_level_from_config(&config.get()));
    start_dev_log_writer(logs.clone());
    logs.log(
        LogLevel::Info,
        "app",
        format!("Integra {APP_VERSION} starting"),
    );

    let (managers_tx, managers_rx) = mpsc::channel::<ManagerEvent>();
    let lists = Arc::new(Mutex::new(ListsManager::new(".")));
    let managers: bridge_store::ManagersMap = Arc::new(Mutex::new(HashMap::new()));

    let store = Arc::new(bridge_store::BridgeStoreImpl {
        config: config.clone(),
        lists: lists.clone(),
        managers: managers.clone(),
        logs: logs.clone(),
    });
    let bridge = BridgeServer::new(store, config.get().bridge.token, config.get().bridge.port);

    let state = Arc::new(AppState {
        config: config.clone(),
        logs: logs.clone(),
        lists,
        managers,
        managers_tx,
        netmon: Netmon::new(),
        bridge,
        benchmark: Mutex::new(None),
        tgproxy: Arc::new(Mutex::new(tgproxy::TgProxyInner::default())),
    });

    if config.get().bridge.enabled {
        match state.bridge.start() {
            Ok(port) => logs.log(
                LogLevel::Success,
                "bridge",
                tr_with("bridge.log_started", &[("port", &port.to_string())]),
            ),
            Err(err) => logs.log(LogLevel::Warn, "bridge", err),
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            // Commands use `State<AppState>`; the Arc clone below feeds the
            // background threads. Both share the same Arc-backed internals.
            app.manage((*state).clone());

            // Background pump: manager events → UI events + logs.
            {
                let handle = handle.clone();
                let logs = logs.clone();
                std::thread::Builder::new()
                    .name("integra-events".into())
                    .spawn(move || {
                        use tauri::Emitter;
                        for event in managers_rx {
                            match event {
                                ManagerEvent::Status {
                                    profile,
                                    status,
                                    pid,
                                } => {
                                    let _ = handle.emit(
                                        "profile-event",
                                        json!({"profile": profile, "status": status, "pid": pid}),
                                    );
                                }
                                ManagerEvent::Log { level, message } => {
                                    logs.log(LogLevel::parse(&level), "zapret", message);
                                }
                                ManagerEvent::Error { profile, message } => {
                                    logs.log(
                                        LogLevel::Error,
                                        "zapret",
                                        format!("[{profile}] {message}"),
                                    );
                                }
                            }
                        }
                    })
                    .ok();
            }

            // Live log stream to the UI.
            {
                let handle = handle.clone();
                logs.subscribe(Arc::new(move |entry: &LogEntry| {
                    use tauri::Emitter;
                    let _ = handle.emit("logs", entry);
                }));
            }

            // Tray icon with a minimal menu.
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
                let show = MenuItem::with_id(&handle, "show", "Integra", true, None::<&str>)?;
                let quit = MenuItem::with_id(&handle, "quit", "Выход / Quit", true, None::<&str>)?;
                let menu = Menu::with_items(&handle, &[&show, &quit])?;
                let mut tray = TrayIconBuilder::with_id("integra-tray")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "show" => show_main_window(app),
                        "quit" => {
                            let state: tauri::State<AppState> = app.state();
                            stop_all_profiles(&state);
                            tgproxy::stop_proxy(state.tgproxy.clone());
                            app.exit(0);
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            show_main_window(tray.app_handle());
                        }
                    });
                if let Some(icon) = app.default_window_icon() {
                    tray = tray.icon(icon.clone());
                }
                tray.build(&handle)?;
            }

            // Window geometry from the config. The window has a fixed size,
            // but it is fitted to the current monitor so it fits any screen
            // (e.g. 1180x760 on FullHD, smaller on 1366x768 laptops).
            {
                let window_config = state.config.get().window;
                if let Some(window) = app.get_webview_window("main") {
                    if let Ok(Some(mon)) = window.current_monitor() {
                        let scale = mon.scale_factor();
                        let (mw, mh) = (
                            mon.size().width as f64 / scale,
                            mon.size().height as f64 / scale,
                        );
                        let w = 1180.0_f64.min(mw * 0.92);
                        let h = 760.0_f64.min(mh * 0.88);
                        let _ = window.set_size(tauri::LogicalSize::new(w, h));
                    }
                    if window_config.maximized {
                        let _ = window.maximize();
                    } else {
                        // Only restore the saved position when it is actually
                        // on a visible screen; otherwise (the window was
                        // hidden/minimized when last saved, or the monitor is
                        // gone) keep the default centered placement.
                        if let (Some(x), Some(y)) = (window_config.x, window_config.y) {
                            let on_screen = window
                                .available_monitors()
                                .map(|mons| {
                                    mons.iter().any(|m| {
                                        let p = m.position();
                                        let s = m.size();
                                        (x >= p.x && x < p.x + s.width as i32)
                                            && (y >= p.y && y < p.y + s.height as i32)
                                    })
                                })
                                .unwrap_or(false);
                            if on_screen {
                                let _ = window
                                    .set_position(tauri::PhysicalPosition::new(x, y));
                            }
                        }
                    }
                }
                if !state.config.get().start_minimized {
                    show_main_window(&handle);
                }
            }

            autostart_profiles(state.clone());

            // Optional zapret update check on startup.
            if state.config.get().auto_check_zapret {
                std::thread::Builder::new()
                    .name("integra-update-check".into())
                    .spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(2500));
                        if let Ok(release) = integra_zapret::maintenance::fetch_latest() {
                            use tauri::Emitter;
                            let _ = handle.emit(
                                "zapret-update",
                                json!({"phase": "check", "release": release}),
                            );
                        }
                    })
                    .ok();
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let state: tauri::State<AppState> = app.state();
                if state.config.get().close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                    state.log_debug("app", "window hidden to tray");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            i18n_bundle,
            get_config,
            set_language,
            set_theme,
            update_settings,
            save_window_state,
            app_info,
            relaunch_as_admin,
            system_check,
            complete_first_run,
            list_profiles,
            add_profile,
            update_profile,
            remove_profile,
            set_active_profile,
            list_strategies,
            start_profile,
            stop_profile,
            profile_status,
            install_service,
            remove_service,
            open_in_explorer,
            reveal_config,
            list_read,
            list_meta,
            list_add,
            list_remove,
            list_replace,
            list_remove_from_all,
            netmon_start,
            netmon_stop,
            netmon_status,
            netmon_entries,
            netmon_summary,
            netmon_processes,
            netmon_export_csv,
            diag_sites,
            diag_add_site,
            diag_remove_site,
            diag_run,
            benchmark_start,
            benchmark_status,
            benchmark_cancel,
            bridge_status,
            bridge_start,
            bridge_stop,
            bridge_pairing,
            bridge_regenerate_code,
            netcheck_run,
            dns_servers,
            dns_benchmark,
            dns_status,
            dns_apply,
            dns_reset,
            roblox_find,
            roblox_hosts_state,
            roblox_hosts_apply,
            roblox_hosts_revert,
            tgproxy_status,
            tgproxy_set_path,
            tgproxy_setup,
            tgproxy_start,
            tgproxy_stop,
            tgproxy_logs,
            logs_snapshot,
            logs_clear,
            logs_export,
            dev_enable,
            dev_disable,
            open_dev_logs,
            theme_presets,
            theme_save_preset,
            theme_delete_preset,
            read_image_data_url,
            save_background_image,
            zapret_check_update,
            zapret_installed_version,
            zapret_install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {});
}
