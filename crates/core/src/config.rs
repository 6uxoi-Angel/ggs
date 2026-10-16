//! Reliable JSON configuration with atomic writes, corruption recovery,
//! portable mode and migration from the legacy Python config (schema v2).

use std::fs;
use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::consts::*;
use crate::profile::ZapretProfile;
use crate::random_hex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub schema: u32,
    pub language: String,
    pub theme: String,

    pub profiles: Vec<ZapretProfile>,
    pub active_profile: Option<String>,

    pub bridge: BridgeConfig,
    pub netmon: NetmonConfig,
    pub diagnostics: DiagnosticsConfig,
    pub window: WindowConfig,

    pub start_minimized: bool,
    pub autostart_minimized: bool,
    pub close_to_tray: bool,
    pub autostart_windows: bool,

    pub log_level: String,

    // Zapret maintenance (implemented in a later iteration, persisted now).
    pub auto_check_zapret: bool,
    pub auto_install_zapret: bool,
    pub auto_update_zapret: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BridgeConfig {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetmonConfig {
    pub poll_interval_ms: u64,
    pub resolve_hosts: bool,
    pub auto_include_new_processes: bool,
    pub max_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnosticsConfig {
    pub custom_sites: Vec<CustomSite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomSite {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub maximized: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema: CURRENT_SCHEMA,
            language: "ru".into(),
            theme: "dark_accent".into(),
            profiles: Vec::new(),
            active_profile: None,
            bridge: BridgeConfig::default(),
            netmon: NetmonConfig::default(),
            diagnostics: DiagnosticsConfig::default(),
            window: WindowConfig::default(),
            start_minimized: false,
            autostart_minimized: true,
            close_to_tray: true,
            autostart_windows: false,
            log_level: "ALL".into(),
            auto_check_zapret: true,
            auto_install_zapret: false,
            auto_update_zapret: false,
        }
    }
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 8765,
            token: String::new(),
        }
    }
}

impl Default for NetmonConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1000,
            resolve_hosts: true,
            auto_include_new_processes: true,
            max_entries: 5000,
        }
    }
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self { custom_sites: Vec::new() }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { width: 1180, height: 760, x: None, y: None, maximized: false }
    }
}

/// Thread-safe configuration handle.
pub struct ConfigStore {
    path: PathBuf,
    data: RwLock<AppConfig>,
}

impl ConfigStore {
    /// Load configuration from `path` (or the platform default location).
    pub fn load(path: Option<PathBuf>) -> Self {
        let path = path.unwrap_or_else(default_config_path);
        let mut data = read_config_file(&path);
        sanitize(&mut data);
        Self { path, data: RwLock::new(data) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> AppConfig {
        self.data.read().clone()
    }

    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&AppConfig) -> R,
    {
        f(&self.data.read())
    }

    /// Mutate and persist atomically. Returns the save result.
    pub fn update<F>(&self, f: F) -> std::io::Result<()>
    where
        F: FnOnce(&mut AppConfig),
    {
        {
            let mut data = self.data.write();
            f(&mut data);
            sanitize(&mut data);
        }
        self.save()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let data = self.data.read().clone();
        atomic_write_json(&self.path, &data)
    }

    /// Active profile (falls back to the first enabled profile).
    pub fn active_profile(&self) -> Option<ZapretProfile> {
        let data = self.data.read();
        let by_id = data
            .active_profile
            .as_ref()
            .and_then(|id| data.profiles.iter().find(|p| &p.id == id));
        by_id
            .or_else(|| data.profiles.iter().find(|p| p.enabled))
            .cloned()
    }
}

pub fn default_config_path() -> PathBuf {
    let exe_dir = crate::paths::application_root();
    if exe_dir.join(PORTABLE_FLAG).exists() {
        return exe_dir.join(CONFIG_FILE);
    }
    config_dir().join(APP_NAME).join(CONFIG_FILE)
}

pub fn config_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata);
        }
    } else if cfg!(target_os = "macos") {
        return dirs_home().join("Library/Application Support");
    } else if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }
    dirs_home().join(".config")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn read_config_file(path: &Path) -> AppConfig {
    if !path.exists() {
        // Try migrating the legacy Python configuration.
        if let Some(config) = migrate_legacy(path) {
            return config;
        }
        return AppConfig::default();
    }
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<AppConfig>(&text) {
            Ok(mut config) => {
                config.schema = CURRENT_SCHEMA;
                config
            }
            Err(_) => {
                // Keep the broken file for inspection, fall back to defaults.
                let broken = path.with_extension("broken.json");
                let _ = fs::rename(path, broken);
                AppConfig::default()
            }
        },
        Err(_) => AppConfig::default(),
    }
}

/// Legacy (Python v2) config layout: flat keys `zapret_path`, `bridge_token`, ...
#[derive(Deserialize)]
struct LegacyConfig {
    #[serde(default)]
    zapret_path: String,
    #[serde(default)]
    bridge_token: String,
    #[serde(default)]
    bridge_port: Option<u16>,
    #[serde(default)]
    bridge_enabled: Option<bool>,
    #[serde(default)]
    selected_strategy: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    start_minimized: Option<bool>,
    #[serde(default)]
    close_to_tray: Option<bool>,
    #[serde(default)]
    autostart_windows: Option<bool>,
    #[serde(default)]
    autostart_zapret: Option<bool>,
    #[serde(default)]
    log_level: Option<String>,
}

fn migrate_legacy(new_path: &Path) -> Option<AppConfig> {
    let candidates = [
        new_path.parent()?.join("config-v2.json"),
        legacy_v2_path()?,
    ];
    let file = candidates.into_iter().find(|p| p.is_file())?;
    let text = fs::read_to_string(&file).ok()?;
    let legacy: LegacyConfig = serde_json::from_str(&text).ok()?;

    let mut config = AppConfig::default();
    if let Some(lang) = legacy.language {
        config.language = lang;
    }
    if let Some(theme) = legacy.theme {
        config.theme = theme;
    }
    if !legacy.bridge_token.trim().is_empty() {
        config.bridge.token = legacy.bridge_token;
    }
    if let Some(port) = legacy.bridge_port {
        config.bridge.port = port;
    }
    if let Some(enabled) = legacy.bridge_enabled {
        config.bridge.enabled = enabled;
    }
    config.start_minimized = legacy.start_minimized.unwrap_or(false);
    config.close_to_tray = legacy.close_to_tray.unwrap_or(true);
    config.autostart_windows = legacy.autostart_windows.unwrap_or(false);
    config.log_level = legacy.log_level.unwrap_or_else(|| "ALL".into());

    let zapret_path = legacy.zapret_path.trim();
    if !zapret_path.is_empty() {
        let mut profile = ZapretProfile::new("Zapret", zapret_path);
        if let Some(strategy) = legacy.selected_strategy {
            profile.selected_strategy = strategy;
        }
        profile.autostart = legacy.autostart_zapret.unwrap_or(false);
        config.active_profile = Some(profile.id.clone());
        config.profiles.push(profile);
    }
    Some(config)
}

fn legacy_v2_path() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        let appdata = std::env::var("APPDATA").ok()?;
        Some(PathBuf::from(appdata).join("Integra").join("config.json"))
    } else {
        None
    }
}

/// Clamp and validate everything a user (or a corrupted file) could set.
fn sanitize(config: &mut AppConfig) {
    if !["ru", "en"].contains(&config.language.as_str()) {
        config.language = "ru".into();
    }
    if !["light", "dark", "dark_accent"].contains(&config.theme.as_str()) {
        config.theme = "dark_accent".into();
    }
    if config.bridge.token.trim().is_empty() {
        config.bridge.token = random_hex(32);
    }
    if config.bridge.port < MIN_PORT {
        config.bridge.port = MIN_PORT;
    }
    if config.netmon.poll_interval_ms < 200 {
        config.netmon.poll_interval_ms = 200;
    }
    if config.netmon.poll_interval_ms > 30_000 {
        config.netmon.poll_interval_ms = 30_000;
    }
    if config.netmon.max_entries == 0 || config.netmon.max_entries > 100_000 {
        config.netmon.max_entries = 5000;
    }
    config.diagnostics.custom_sites.truncate(MAX_CUSTOM_SITES);

    // Unique profile ids; drop active pointer if it dangles.
    let mut seen = std::collections::HashSet::new();
    config.profiles.retain_mut(|p| {
        if !seen.insert(p.id.clone()) {
            p.id = random_hex(4);
        }
        if p.name.trim().is_empty() {
            p.name = p.path.to_string_lossy().to_string();
        }
        true
    });
    if let Some(active) = &config.active_profile {
        if !config.profiles.iter().any(|p| &p.id == active) {
            config.active_profile = None;
        }
    }
    if config.active_profile.is_none() {
        config.active_profile = config.profiles.first().map(|p| p.id.clone());
    }
    config.schema = CURRENT_SCHEMA;
}

pub fn atomic_write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("integra-cfg-{}-{}", name, random_hex(4)))
    }

    #[test]
    fn roundtrip_and_generated_token() {
        let path = tmp("roundtrip");
        let store = ConfigStore::load(Some(path.clone()));
        assert!(!store.get().bridge.token.is_empty(), "token must be generated");
        store
            .update(|c| {
                c.theme = "light".into();
                c.bridge.port = 9001;
                c.profiles.push(ZapretProfile::new("Main", "/opt/zapret"));
            })
            .unwrap();

        let reloaded = ConfigStore::load(Some(path.clone()));
        let config = reloaded.get();
        assert_eq!(config.theme, "light");
        assert_eq!(config.bridge.port, 9001);
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(config.active_profile.as_deref(), Some(config.profiles[0].id.as_str()));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn port_clamped_into_range() {
        let path = tmp("port");
        let store = ConfigStore::load(Some(path.clone()));
        store.update(|c| c.bridge.port = 10).unwrap();
        assert_eq!(store.get().bridge.port, MIN_PORT);
        fs::remove_file(&path).ok();
    }

    #[test]
    fn recovers_broken_json() {
        let path = tmp("broken");
        fs::write(&path, "{broken").unwrap();
        let store = ConfigStore::load(Some(path.clone()));
        assert_eq!(store.get().theme, "dark_accent");
        assert!(path.with_extension("broken.json").exists());
        fs::remove_file(path.with_extension("broken.json")).ok();
    }

    #[test]
    fn migrates_legacy_python_config() {
        let dir = tmp("legacy-dir");
        fs::create_dir_all(&dir).unwrap();
        let legacy_path = dir.join("config-v2.json");
        fs::write(
            &legacy_path,
            r#"{
                "zapret_path": "C:\\zapret",
                "selected_strategy": "general FAKE TLS.bat",
                "bridge_token": "deadbeef",
                "bridge_port": 9001,
                "language": "en",
                "theme": "dark",
                "autostart_zapret": true
            }"#,
        )
        .unwrap();
        let new_path = dir.join("config.json");
        let store = ConfigStore::load(Some(new_path));
        let config = store.get();
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(config.profiles[0].selected_strategy, "general FAKE TLS.bat");
        assert!(config.profiles[0].autostart);
        assert_eq!(config.bridge.token, "deadbeef");
        assert_eq!(config.bridge.port, 9001);
        assert_eq!(config.language, "en");
        assert_eq!(config.theme, "dark");
        fs::remove_dir_all(&dir).ok();
    }
}
