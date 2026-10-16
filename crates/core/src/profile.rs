//! Zapret installation profile: one entry per managed zapret folder.
//!
//! Integra 3 supports several independent installations at once — each with
//! its own strategies, lists, running process and status.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::random_hex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ZapretProfile {
    /// Stable identifier (8 hex chars).
    pub id: String,
    /// User-visible name, e.g. "Main" or "Discord mod".
    pub name: String,
    /// Filesystem path of the zapret installation.
    pub path: PathBuf,
    /// Last selected strategy file name (`general*.bat`).
    #[serde(default = "default_strategy")]
    pub selected_strategy: String,
    /// Start this profile's strategy automatically with the app.
    #[serde(default)]
    pub autostart: bool,
    /// Profile participates in status polling / UI.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Remembered service state hint (the real check is done via `sc query`).
    #[serde(default)]
    pub service_expected: bool,
}

fn default_strategy() -> String {
    "general.bat".to_string()
}
fn yes() -> bool {
    true
}

impl ZapretProfile {
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            id: random_hex(4),
            name: name.into(),
            path: path.into(),
            selected_strategy: default_strategy(),
            autostart: false,
            enabled: true,
            service_expected: false,
        }
    }

    /// Cheap structural validation: does the folder look like a zapret install?
    pub fn inspect(&self) -> ProfileHealth {
        let path = Path::new(&self.path);
        if self.path.as_os_str().is_empty() {
            return ProfileHealth::MissingPath;
        }
        if !path.is_dir() {
            return ProfileHealth::MissingFolder;
        }
        let has_bat = crate::lists::has_strategies(path);
        let has_winws = path.join("bin").join("winws.exe").is_file() || path.join("winws.exe").is_file();
        let has_lists = path.join("lists").is_dir();
        match (has_bat, has_winws) {
            (true, true) => ProfileHealth::Ready { lists: has_lists },
            (false, true) => ProfileHealth::NoStrategies,
            (true, false) => ProfileHealth::NoWinws,
            (false, false) => ProfileHealth::NotAZapretFolder,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "detail")]
pub enum ProfileHealth {
    Ready { lists: bool },
    MissingPath,
    MissingFolder,
    NoStrategies,
    NoWinws,
    NotAZapretFolder,
}

impl ProfileHealth {
    pub fn ok(&self) -> bool {
        matches!(self, ProfileHealth::Ready { .. })
    }

    pub fn i18n_key(&self) -> &'static str {
        match self {
            ProfileHealth::Ready { .. } => "profile.health_ready",
            ProfileHealth::MissingPath => "profile.health_missing_path",
            ProfileHealth::MissingFolder => "profile.health_missing_folder",
            ProfileHealth::NoStrategies => "profile.health_no_strategies",
            ProfileHealth::NoWinws => "profile.health_no_winws",
            ProfileHealth::NotAZapretFolder => "profile.health_not_zapret",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn inspect_detects_layout() {
        let tmp = std::env::temp_dir().join(format!("integra-profile-{}", random_hex(4)));
        fs::create_dir_all(tmp.join("lists")).unwrap();
        fs::write(tmp.join("general.bat"), "@echo off\r\n").unwrap();

        let mut profile = ZapretProfile::new("test", tmp.clone());
        assert_eq!(profile.inspect(), ProfileHealth::NoWinws);

        fs::write(tmp.join("winws.exe"), b"stub").unwrap();
        assert_eq!(profile.inspect(), ProfileHealth::Ready { lists: true });

        profile.path = tmp.join("nonexistent");
        assert_eq!(profile.inspect(), ProfileHealth::MissingFolder);

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn ids_are_unique() {
        let a = ZapretProfile::new("a", "/tmp/a");
        let b = ZapretProfile::new("b", "/tmp/b");
        assert_ne!(a.id, b.id);
    }
}
