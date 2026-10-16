//! Zapret control: strategy discovery, hidden runtime BAT generation,
//! winws.exe process lifecycle and Windows service management.
//!
//! Multi-profile by design: every [`ZapretManager`] owns exactly one zapret
//! installation and only ever terminates the `winws.exe` processes it started
//! (tracked through the launcher's child PID), so several profiles can run
//! side by side.

pub mod manager;
pub mod runtime_bat;
pub mod strategy;
pub mod version;

#[cfg(windows)]
pub mod win;

pub use manager::{ManagerEvent, Status, ZapretManager};
pub use strategy::{Strategy, StrategyCategory};
pub use version::Version;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ZapretError {
    #[error("zapret.windows_only")]
    WindowsOnly,
    #[error("zapret.not_admin")]
    NotAdmin,
    #[error("zapret.path_missing")]
    PathMissing,
    #[error("zapret.no_bat")]
    NoStrategies,
    #[error("zapret.no_winws")]
    NoWinws,
    #[error("zapret.service_conflict")]
    ServiceConflict,
    #[error("zapret.strategy_missing: {0}")]
    StrategyMissing(String),
    #[error("zapret.exit_code")]
    LauncherExited { code: i32, output: String },
    #[error("zapret.timeout")]
    Timeout(String),
    #[error("zapret.start_failed")]
    StartFailed(String),
    #[error("zapret.stop_partial")]
    StopPartial(String),
    #[error("zapret.install_failed")]
    InstallFailed(String),
    #[error("zapret.remove_failed")]
    RemoveFailed(String),
    #[error("windows.batch_no_winws")]
    BatchInvalid,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ZapretError {
    pub fn i18n_key(&self) -> &'static str {
        match self {
            ZapretError::WindowsOnly => "zapret.windows_only",
            ZapretError::NotAdmin => "zapret.not_admin",
            ZapretError::PathMissing => "zapret.path_missing",
            ZapretError::NoStrategies => "zapret.no_bat",
            ZapretError::NoWinws => "zapret.no_winws",
            ZapretError::ServiceConflict => "zapret.service_conflict",
            ZapretError::StrategyMissing(_) => "zapret.strategy_missing",
            ZapretError::LauncherExited { .. } => "zapret.exit_code",
            ZapretError::Timeout(_) => "zapret.timeout",
            ZapretError::StartFailed(_) => "zapret.start_failed",
            ZapretError::StopPartial(_) => "zapret.stop_partial",
            ZapretError::InstallFailed(_) => "zapret.install_failed",
            ZapretError::RemoveFailed(_) => "zapret.remove_failed",
            ZapretError::BatchInvalid => "windows.batch_no_winws",
            ZapretError::Io(_) => "common.error",
        }
    }
}
