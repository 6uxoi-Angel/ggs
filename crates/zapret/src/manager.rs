//! Per-profile zapret lifecycle manager.
//!
//! One manager == one zapret installation. It never touches winws processes
//! belonging to other profiles (children are matched by launcher PID).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

use integra_core::i18n::tr_with;
use integra_core::lists::{glob_general_bat, natural_key};

use crate::runtime_bat::build_hidden_runtime_batch;
use crate::strategy::{discover, Strategy};
use crate::ZapretError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Stopped,
    Running,
    Service,
}

#[derive(Debug, Clone)]
pub enum ManagerEvent {
    Status { profile: String, status: Status, pid: u32 },
    Log { level: String, message: String },
    Error { profile: String, message: String },
}

struct Runtime {
    launcher: Child,
    bat: PathBuf,
    log: PathBuf,
    strategy: String,
    winws_pid: Option<u32>,
}

pub struct ZapretManager {
    profile_id: String,
    path: Mutex<PathBuf>,
    runtime: Mutex<Option<Runtime>>,
    service_cache: Mutex<(Instant, bool, bool)>,
    events: Mutex<Option<Sender<ManagerEvent>>>,
}

impl ZapretManager {
    pub fn new(profile_id: impl Into<String>, zapret_path: impl Into<PathBuf>) -> Self {
        Self {
            profile_id: profile_id.into(),
            path: Mutex::new(zapret_path.into()),
            runtime: Mutex::new(None),
            service_cache: Mutex::new((Instant::now() - Duration::from_secs(60), false, false)),
            events: Mutex::new(None),
        }
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    pub fn path(&self) -> PathBuf {
        self.path.lock().clone()
    }

    pub fn set_path(&self, path: impl Into<PathBuf>) {
        *self.path.lock() = path.into();
    }

    pub fn attach_events(&self, tx: Sender<ManagerEvent>) {
        *self.events.lock() = Some(tx);
    }

    fn emit(&self, event: ManagerEvent) {
        if let Some(tx) = self.events.lock().as_ref() {
            let _ = tx.send(event);
        }
    }

    fn log(&self, level: &str, message: String) {
        self.emit(ManagerEvent::Log { level: level.to_string(), message });
    }

    // ── validation & discovery ───────────────────────────────

    pub fn winws_path(&self) -> Option<PathBuf> {
        let path = self.path.lock();
        for candidate in [path.join("bin").join("winws.exe"), path.join("winws.exe")] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    /// Pure filesystem validation (works on any OS, used by tests and UI).
    pub fn validate(&self) -> Result<(), ZapretError> {
        let path = self.path.lock().clone();
        if !path.is_dir() {
            return Err(ZapretError::PathMissing);
        }
        if glob_general_bat(&path).next().is_none() {
            return Err(ZapretError::NoStrategies);
        }
        if self.winws_path().is_none() {
            return Err(ZapretError::NoWinws);
        }
        Ok(())
    }

    pub fn strategies(&self) -> Vec<Strategy> {
        discover(&self.path.lock())
    }

    fn runtime_bat_path(&self) -> PathBuf {
        self.path.lock().join(format!(".integra-runtime-{}.bat", self.profile_id))
    }

    // ── start / stop ─────────────────────────────────────────

    pub fn start(&self, strategy_file: &str) -> Result<u32, ZapretError> {
        #[cfg(not(windows))]
        let _ = strategy_file;
        #[cfg(not(windows))]
        return Err(ZapretError::WindowsOnly);

        #[cfg(windows)]
        {
            self.validate()?;
            if !crate::win::is_admin() {
                return Err(ZapretError::NotAdmin);
            }
            let (installed, _) = self.service_state(true);
            if installed {
                return Err(ZapretError::ServiceConflict);
            }

            let path = self.path.lock().clone();
            let strategy_name = Path::new(strategy_file)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let bat = path.join(&strategy_name);
            if !strategy_name.to_lowercase().ends_with(".bat") || !bat.is_file() {
                return Err(ZapretError::StrategyMissing(strategy_name));
            }

            let _ = self.stop_inner(false);

            let runtime_bat = self.runtime_bat_path();
            let _ = std::fs::remove_file(&runtime_bat);
            build_hidden_runtime_batch(&bat, &runtime_bat)?;

            let log_path = integra_core::paths::logs_dir()
                .join(format!("zapret-launch-{}.log", self.profile_id));
            if let Some(parent) = log_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let log_file = std::fs::File::create(&log_path)?;
            let log_err = log_file.try_clone()?;

            use std::os::windows::process::CommandExt;
            let mut command = Command::new("cmd.exe");
            command
                .args(["/d", "/q", "/c", "call"])
                .arg(&runtime_bat)
                .current_dir(&path)
                .env("NO_UPDATE_CHECK", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::from(log_file))
                .stderr(Stdio::from(log_err))
                .creation_flags(crate::win::CREATE_NO_WINDOW_FLAG);
            let launcher = command.spawn().map_err(|e| ZapretError::StartFailed(e.to_string()))?;
            let launcher_pid = launcher.id();

            let mut runtime_guard = self.runtime.lock();
            *runtime_guard = Some(Runtime {
                launcher,
                bat: runtime_bat,
                log: log_path.clone(),
                strategy: strategy_name.clone(),
                winws_pid: None,
            });
            drop(runtime_guard);

            // Wait up to 7 seconds for winws.exe (child of our launcher).
            let deadline = Instant::now() + Duration::from_secs(7);
            let mut winws_pid = None;
            while Instant::now() < deadline {
                let pids = crate::win::find_winws(Some(launcher_pid));
                if let Some(pid) = pids.first() {
                    winws_pid = Some(*pid);
                    break;
                }
                let exited = {
                    let mut guard = self.runtime.lock();
                    guard
                        .as_mut()
                        .and_then(|r| r.launcher.try_wait().ok())
                        .flatten()
                        .map(|s| s.code().unwrap_or(-1))
                };
                if let Some(code) = exited {
                    let output = read_log_tail(&log_path, 1800);
                    let _ = self.stop_inner(false);
                    return Err(ZapretError::LauncherExited { code, output });
                }
                std::thread::sleep(Duration::from_millis(150));
            }

            let Some(pid) = winws_pid else {
                let output = read_log_tail(&log_path, 1800);
                let _ = self.stop_inner(false);
                return Err(ZapretError::Timeout(output));
            };

            if let Some(runtime) = self.runtime.lock().as_mut() {
                runtime.winws_pid = Some(pid);
            }
            self.emit(ManagerEvent::Status { profile: self.profile_id.clone(), status: Status::Running, pid });
            self.log(
                "SUCCESS",
                tr_with("zapret.started", &[("strategy", &strategy_name), ("pid", &pid.to_string())]),
            );
            Ok(pid)
        }
    }

    pub fn stop(&self) -> Result<u32, ZapretError> {
        self.stop_inner(true)
    }

    fn stop_inner(&self, report_when_idle: bool) -> Result<u32, ZapretError> {
        let runtime = self.runtime.lock().take();
        let Some(mut runtime) = runtime else {
            if report_when_idle {
                self.log("INFO", integra_core::i18n::tr("zapret.already_stopped"));
            }
            return Ok(0);
        };

        let mut killed = 0u32;
        #[cfg(windows)]
        {
            let launcher_pid = runtime.launcher.id();
            for pid in crate::win::find_winws(Some(launcher_pid)) {
                if crate::win::terminate(pid).is_ok() {
                    killed += 1;
                }
            }
            let _ = runtime.launcher.kill();
            let _ = runtime.launcher.wait();
        }
        #[cfg(not(windows))]
        {
            let _ = runtime.launcher.kill();
            let _ = &mut runtime;
        }

        let _ = std::fs::remove_file(&runtime.bat);
        self.emit(ManagerEvent::Status { profile: self.profile_id.clone(), status: Status::Stopped, pid: 0 });
        if killed > 0 {
            self.log("INFO", tr_with("zapret.stopped_count", &[("count", &killed.to_string())]));
        } else if report_when_idle {
            self.log("INFO", integra_core::i18n::tr("zapret.already_stopped"));
        }
        Ok(killed)
    }

    // ── status ───────────────────────────────────────────────

    pub fn pid(&self) -> u32 {
        #[cfg(windows)]
        {
            if let Some(runtime) = self.runtime.lock().as_ref() {
                if let Some(pid) = runtime.winws_pid {
                    if crate::win::process_alive(pid) {
                        return pid;
                    }
                }
            }
            let (_, running) = self.service_state(false);
            if running {
                return crate::win::find_winws(None).first().copied().unwrap_or(0);
            }
        }
        0
    }

    pub fn status(&self) -> Status {
        #[cfg(windows)]
        {
            let own_running = self
                .runtime
                .lock()
                .as_ref()
                .and_then(|r| r.winws_pid)
                .is_some_and(|pid| crate::win::process_alive(pid));
            if own_running {
                return Status::Running;
            }
            let (installed, running) = self.service_state(false);
            if installed && running {
                return Status::Service;
            }
        }
        Status::Stopped
    }

    pub fn current_strategy(&self) -> Option<String> {
        self.runtime.lock().as_ref().map(|r| r.strategy.clone())
    }

    /// (installed, running) with a 5-second cache — `sc query` is not free.
    pub fn service_state(&self, force: bool) -> (bool, bool) {
        #[cfg(not(windows))]
        {
            let _ = force;
            return (false, false);
        }
        #[cfg(windows)]
        {
            let mut cache = self.service_cache.lock();
            if !force && cache.0.elapsed() < Duration::from_secs(5) {
                return (cache.1, cache.2);
            }
            let (installed, running) = crate::win::query_service();
            *cache = (Instant::now(), installed, running);
            (installed, running)
        }
    }

    // ── service install / remove ─────────────────────────────

    pub fn install_service(&self, strategy_file: &str) -> Result<(), ZapretError> {
        #[cfg(not(windows))]
        {
            let _ = strategy_file;
            Err(ZapretError::WindowsOnly)
        }
        #[cfg(windows)]
        {
            self.validate()?;
            if !crate::win::is_admin() {
                return Err(ZapretError::NotAdmin);
            }
            let path = self.path.lock().clone();
            let strategy_name = Path::new(strategy_file)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .ok_or_else(|| ZapretError::StrategyMissing(strategy_file.to_string()))?;

            // A leftover runtime file must not appear in the service.bat menu.
            let _ = std::fs::remove_file(self.runtime_bat_path());

            let output = if path.join("service_install.bat").is_file() {
                crate::win::run_hidden(
                    "cmd.exe",
                    &["/d", "/q", "/c", "call", "service_install.bat", &strategy_name],
                    Some(&path),
                    None,
                    Duration::from_secs(150),
                )?
            } else if path.join("service.bat").is_file() {
                let mut candidates: Vec<String> = std::fs::read_dir(&path)
                    .map_err(ZapretError::Io)?
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| {
                        let lower = n.to_lowercase();
                        lower.ends_with(".bat")
                            && !lower.starts_with("service")
                            && !lower.starts_with(".integra-")
                    })
                    .collect();
                candidates.sort_by_key(|n| natural_key(n));
                let Some(index) = candidates.iter().position(|n| n == &strategy_name) else {
                    return Err(ZapretError::StrategyMissing(strategy_name));
                };
                // 1 = install, strategy index, Enter (pause), 0 = exit.
                let scripted = format!("1\r\n{}\r\n\r\n0\r\n", index + 1);
                crate::win::run_hidden(
                    "cmd.exe",
                    &["/d", "/q", "/c", "call", "service.bat", "admin"],
                    Some(&path),
                    Some(&scripted),
                    Duration::from_secs(180),
                )?
            } else {
                return Err(ZapretError::InstallFailed("service.bat not found".into()));
            };

            let (installed, running) = self.service_state(true);
            if !installed {
                let details = if !output.stderr.trim().is_empty() {
                    output.stderr.trim()
                } else {
                    output.stdout.trim()
                };
                let details = details.chars().rev().take(1200).collect::<Vec<_>>();
                let details: String = details.into_iter().rev().collect();
                return Err(ZapretError::InstallFailed(details));
            }

            let status = if running { Status::Service } else { Status::Stopped };
            self.emit(ManagerEvent::Status { profile: self.profile_id.clone(), status, pid: self.pid() });
            let running_label = if running { integra_core::i18n::tr("zapret.and_started") } else { String::new() };
            self.log("SUCCESS", tr_with("zapret.service_installed", &[("running", &running_label)]));
            Ok(())
        }
    }

    pub fn remove_service(&self) -> Result<(), ZapretError> {
        #[cfg(not(windows))]
        return Err(ZapretError::WindowsOnly);

        #[cfg(windows)]
        {
            if !crate::win::is_admin() {
                return Err(ZapretError::NotAdmin);
            }
            crate::win::sc("stop", "zapret");
            crate::win::taskkill_image("winws.exe");
            crate::win::sc("delete", "zapret");
            for service in ["WinDivert", "WinDivert14"] {
                crate::win::sc("stop", service);
                crate::win::sc("delete", service);
            }

            let mut installed = true;
            for _ in 0..10 {
                (installed, _) = self.service_state(true);
                if !installed {
                    break;
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            if installed {
                return Err(ZapretError::RemoveFailed(integra_core::i18n::tr("zapret.remove_retry")));
            }
            self.emit(ManagerEvent::Status { profile: self.profile_id.clone(), status: Status::Stopped, pid: 0 });
            self.log("INFO", integra_core::i18n::tr("zapret.service_removed"));
            Ok(())
        }
    }
}

#[cfg(windows)]
fn read_log_tail(path: &Path, limit: usize) -> String {
    let Ok(bytes) = std::fs::read(path) else { return String::new() };
    let start = bytes.len().saturating_sub(limit);
    decode_tail(&bytes[start..])
}

#[cfg(not(windows))]
fn read_log_tail(_path: &Path, _limit: usize) -> String {
    String::new()
}

/// Decode console output trying UTF-8 then cp866/cp1251 (Russian consoles).
pub fn decode_tail(raw: &[u8]) -> String {
    let cleaned: Vec<u8> = raw.iter().copied().filter(|&b| b != 0).collect();
    let text = match String::from_utf8(cleaned.clone()) {
        Ok(text) => text,
        Err(_) => {
            let (cow, _, _) = encoding_rs::IBM866.decode(&cleaned);
            cow.into_owned()
        }
    };
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use integra_core::lists::tempfile_lite::TempDir;

    fn fake_install(dir: &TempDir) {
        std::fs::create_dir_all(dir.path().join("bin")).unwrap();
        std::fs::write(dir.path().join("general.bat"), "@echo off\r\nstart \"z\" /min \"bin\\winws.exe\" --x\r\n").unwrap();
        std::fs::write(dir.path().join("bin").join("winws.exe"), b"stub").unwrap();
    }

    #[test]
    fn validate_reports_layout_problems() {
        let dir = TempDir::new("integra-mgr");
        let mgr = ZapretManager::new("p1", dir.path());
        assert!(matches!(mgr.validate(), Err(ZapretError::NoStrategies)));

        std::fs::write(dir.path().join("general.bat"), "@echo off").unwrap();
        assert!(matches!(mgr.validate(), Err(ZapretError::NoWinws)));

        fake_install(&dir);
        assert!(mgr.validate().is_ok());
        assert_eq!(mgr.strategies().len(), 1);
    }

    #[test]
    fn control_requires_windows() {
        let dir = TempDir::new("integra-mgr2");
        fake_install(&dir);
        let mgr = ZapretManager::new("p2", dir.path());
        if cfg!(not(windows)) {
            assert!(matches!(mgr.start("general.bat"), Err(ZapretError::WindowsOnly)));
            assert!(matches!(mgr.install_service("general.bat"), Err(ZapretError::WindowsOnly)));
            assert!(matches!(mgr.remove_service(), Err(ZapretError::WindowsOnly)));
            assert_eq!(mgr.status(), Status::Stopped);
            assert_eq!(mgr.service_state(true), (false, false));
        }
    }

    #[test]
    fn stop_when_idle_is_quiet() {
        let dir = TempDir::new("integra-mgr3");
        let mgr = ZapretManager::new("p3", dir.path());
        assert_eq!(mgr.stop().unwrap(), 0);
    }

    #[test]
    fn decode_tail_filters_nuls_and_empty_lines() {
        let raw = b"line1\r\n\x00\x00\r\n\r\nline2\r\n";
        assert_eq!(decode_tail(raw), "line1 | line2");
    }
}
