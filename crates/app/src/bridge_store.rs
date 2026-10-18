//! glue between the HTTP bridge and the app: config, lists, managers, logs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};

use integra_bridge::BridgeStore;
use integra_core::config::AppConfig;
use integra_core::lists::ListSpec;
use integra_core::{ConfigStore, ListsManager, LogBuffer, LogLevel, LISTS};
use integra_zapret::{Status, ZapretManager};

/// Where shared (cross-profile) lists live.
pub fn shared_lists_root() -> PathBuf {
    integra_core::paths::state_dir().join("shared")
}

pub type ManagersMap = Arc<Mutex<HashMap<String, Arc<ZapretManager>>>>;

pub struct BridgeStoreImpl {
    pub config: Arc<ConfigStore>,
    pub lists: Arc<Mutex<ListsManager>>,
    pub managers: ManagersMap,
    pub logs: Arc<LogBuffer>,
}

impl BridgeStoreImpl {
    fn resolve_profile(&self, profile: Option<&str>) -> Option<integra_core::ZapretProfile> {
        let config = self.config.get();
        match profile {
            Some(id) => config.profiles.into_iter().find(|p| p.id == id),
            None => config.active_profile(),
        }
    }

    fn with_root<T>(
        &self,
        profile: Option<&str>,
        f: impl FnOnce(&ListsManager) -> T,
    ) -> Result<T, String> {
        let profile = self
            .resolve_profile(profile)
            .ok_or_else(|| integra_core::i18n::tr("dashboard.no_profiles"))?;
        let root = if profile.use_shared_lists {
            shared_lists_root()
        } else {
            profile.path
        };
        let lists = self.lists.lock();
        lists.set_root(root);
        Ok(f(&lists))
    }
}

impl BridgeStore for BridgeStoreImpl {
    fn status(&self) -> Value {
        let config = self.config.get();
        let active = config.active_profile();
        let mut running = false;
        let mut strategy = Value::Null;
        let mut profile_name = Value::Null;
        if let Some(profile) = &active {
            profile_name = json!(profile.name);
            if let Some(manager) = self.managers.lock().get(&profile.id) {
                let status = manager.status();
                running = matches!(status, Status::Running | Status::Service);
                strategy = json!(manager.current_strategy().unwrap_or_else(|| profile.selected_strategy.clone()));
            } else {
                strategy = json!(profile.selected_strategy);
            }
        }
        json!({
            "zapret_running": running,
            "profile": profile_name,
            "strategy": strategy,
        })
    }

    fn profiles(&self) -> Value {
        let config = self.config.get();
        json!(config
            .profiles
            .iter()
            .map(|p| json!({"id": p.id, "name": p.name, "active": config.active_profile.as_deref() == Some(p.id.as_str())}))
            .collect::<Vec<_>>())
    }

    fn counts(&self, profile: Option<&str>) -> Value {
        let mut map = serde_json::Map::new();
        let _ = self.with_root(profile, |lists| {
            for spec in LISTS {
                map.insert(spec.id.to_string(), json!(lists.count(spec.id)));
            }
        });
        Value::Object(map)
    }

    fn contains(&self, profile: Option<&str>, domain: &str) -> (bool, bool) {
        self.with_root(profile, |lists| {
            (
                lists.contains("list-general", domain),
                lists.contains("list-exclude", domain),
            )
        })
        .unwrap_or((false, false))
    }

    fn add_domains(&self, profile: Option<&str>, mode: &str, domains: &[String]) -> Result<Value, String> {
        let list_id = match mode {
            "bypass" => "list-general",
            "exclude" => "list-exclude",
            other => return Err(integra_core::i18n::tr("bridge.bad_mode").replace("bypass или exclude", other)),
        };
        self.with_root(profile, |lists| lists.add(list_id, domains).map_err(|e| e.to_string()))
            .map_err(|e| e)?
    }

    fn remove_domain(&self, profile: Option<&str>, domain: &str) -> Result<Vec<String>, String> {
        self.with_root(profile, |lists| {
            lists.remove_from_all(domain).map_err(|e| e.to_string())
        })
        .map_err(|e| e)?
    }

    fn measure_latency(&self, host: &str) -> (bool, Option<u64>, Option<String>) {
        integra_diag::measure_tcp_latency(host, 443, std::time::Duration::from_secs(2))
    }

    fn log(&self, level: &str, message: &str) {
        self.logs.log(LogLevel::parse(level), "bridge", message);
    }
}

/// Resolve the effective lists root for a profile (shared or individual).
pub fn lists_root_for(config: &AppConfig, profile_id: &str) -> Result<PathBuf, String> {
    let profile = config
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| integra_core::i18n::tr("zapret.path_missing"))?;
    Ok(if profile.use_shared_lists {
        shared_lists_root()
    } else {
        profile.path.clone()
    })
}

pub fn list_spec(id: &str) -> Option<&'static ListSpec> {
    integra_core::lists::find_spec(id)
}
