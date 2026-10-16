//! Integra core: platform-independent building blocks shared by every crate.
//!
//! * [`config`]  — atomic JSON configuration with multi-profile support and
//!   migration from the legacy Python (v2) configuration.
//! * [`domain`]  — URL/host/IP normalization (IDNA aware).
//! * [`lists`]   — zapret domain/IP list editing with atomic writes.
//! * [`profile`] — the zapret installation profile model (multiple folders).
//! * [`i18n`]    — RU/EN translations shared by backend and frontend.
//! * [`paths`]   — application/resource path resolution (source & packaged).
//! * [`logbuf`]  — in-memory ring log buffer with subscriptions.

pub mod config;
pub mod domain;
pub mod i18n;
pub mod lists;
pub mod logbuf;
pub mod paths;
pub mod profile;

pub use config::{AppConfig, ConfigStore};
pub use domain::{normalize_host, DomainError};
pub use lists::{ListsManager, LISTS};
pub use logbuf::{LogBuffer, LogEntry, LogLevel};
pub use profile::ZapretProfile;

/// Application-wide constants.
pub mod consts {
    pub const APP_NAME: &str = "Integra";
    pub const CONFIG_FILE: &str = "config.json";
    pub const PORTABLE_FLAG: &str = "portable.flag";
    pub const CURRENT_SCHEMA: u32 = 3;
    pub const MIN_PORT: u16 = 1024;
    pub const MAX_CUSTOM_SITES: usize = 40;
}

/// Generate a cryptographically secure lowercase hex token.
pub fn random_hex(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}
