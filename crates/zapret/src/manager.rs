//! Per-profile zapret lifecycle manager.
//!
//! One manager == one zapret installation. It never touches winws processes
//! belonging to other profiles: children are matched by launcher PID and
//! terminated through owned OS handles (`OwnedProcess`), so a recycled PID
//! can never make us kill an unrelated process.
//!
//! Start/stop are serialized by a small phase state machine (`Idle → Starting
//! → Running → Stopping`); a stop requested mid-start sets `pending_stop` and
//! the start loop aborts itself cleanly.
//!
//! NOTE: [`ZapretManager::start`], [`ZapretManager::stop`],
//! [`ZapretManager::install_service`] and [`ZapretManager::remove_service`]
//! are blocking (polling loops, `sc` waits). Async callers (tauri commands)
//! must offload them via `tauri::async_runtime::spawn_blocking` so no
//! `std::thread::sleep` loop ever runs on the async runtime thread.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

use integra_core::i18n::tr_with;
use integra_core::lists::glob_general_bat;

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
    Status {
        profile: String,
        status: Status,
        pid: u32,
    },
    Log {
        level: String,
        message: String,
    },
    Error {
        profile: String,
        message: String,
    },
}

/// Lifecycle phase of the managed installation (start/stop race guard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Starting,
    Running,
    Stopping,
}

/// Outcome of admitting a stop request against the current [`Phase`].
enum StopAdmission {
    /// Phase went Running → Stopping: caller runs the stop and `finish_stop`.
    Began,
    /// A start is in flight: `pending_stop` was set, the start loop aborts
    /// itself (terminate + cleanup + Idle).
    WhileStarting,
    /// Nothing running or a stop is already in progress.
    Idle,
}

struct Runtime {
    launcher: Child,
    bat: PathBuf,
    strategy: String,
    /// Owned handle to the discovered winws.exe child. Killing and liveness
    /// checks go through this handle so PID reuse is harmless.
    #[cfg(windows)]
    winws: Option<crate::win::OwnedProcess>,
}

pub struct ZapretManager {
    profile_id: String,
    path: Mutex<PathBuf>,
    runtime: Mutex<Option<Runtime>>,
    service_cache: Mutex<(Instant, bool, bool)>,
    events: Mutex<Option<Sender<ManagerEvent>>>,
    /// Start/stop phase machine (see [`Phase`]).
    phase: Mutex<Phase>,
    /// Set when stop is requested while a start is still in flight; the
    /// start routine checks it between its wait iterations.
    pending_stop: AtomicBool,
}

impl ZapretManager {
    pub fn new(profile_id: impl Into<String>, zapret_path: impl Into<PathBuf>) -> Self {
        Self {
            profile_id: profile_id.into(),
            path: Mutex::new(zapret_path.into()),
            runtime: Mutex::new(None),
            service_cache: Mutex::new((Instant::now() - Duration::from_secs(60), false, false)),
            events: Mutex::new(None),
            phase: Mutex::new(Phase::Idle),
            pending_stop: AtomicBool::new(false),
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
        self.emit(ManagerEvent::Log {
            level: level.to_string(),
            message,
        });
    }

    // ── phase state machine (start/stop race guard) ──────────

    /// Enter `Starting` — only from `Idle`. Any other phase is rejected so
    /// concurrent starts can never race each other.
    fn try_begin_start(&self) -> Result<(), ZapretError> {
        let mut phase = self.phase.lock();
        match *phase {
            Phase::Idle => {
                self.pending_stop.store(false, Ordering::SeqCst);
                *phase = Phase::Starting;
                Ok(())
            }
            Phase::Starting => Err(ZapretError::StartFailed(
                "start already in progress for this profile".into(),
            )),
            Phase::Running => Err(ZapretError::StartFailed(
                "already running — stop the profile before starting again".into(),
            )),
            Phase::Stopping => Err(ZapretError::StartFailed(
                "stop in progress — retry in a moment".into(),
            )),
        }
    }

    /// Leave `Starting`: on success → `Running`, on failure → `Idle`. Any
    /// early error in the start routine must end here so the phase never
    /// stays stuck on `Starting`.
    fn finish_start(&self, ok: bool) {
        let mut phase = self.phase.lock();
        if *phase == Phase::Starting {
            *phase = if ok { Phase::Running } else { Phase::Idle };
        }
        if !ok {
            self.pending_stop.store(false, Ordering::SeqCst);
        }
    }

    /// Admit a stop request. From `Running` the phase moves to `Stopping`;
    /// from `Starting` only the `pending_stop` flag is raised and the start
    /// loop performs the actual teardown.
    fn try_begin_stop(&self) -> StopAdmission {
        let mut phase = self.phase.lock();
        match *phase {
            Phase::Running => {
                *phase = Phase::Stopping;
                StopAdmission::Began
            }
            Phase::Starting => {
                self.pending_stop.store(true, Ordering::SeqCst);
                StopAdmission::WhileStarting
            }
            Phase::Stopping | Phase::Idle => StopAdmission::Idle,
        }
    }

    /// Leave `Stopping` → `Idle` and drop any pending stop request.
    fn finish_stop(&self) {
        let mut phase = self.phase.lock();
        if *phase == Phase::Stopping {
            *phase = Phase::Idle;
        }
        self.pending_stop.store(false, Ordering::SeqCst);
    }

    // ── validation & discovery ───────────────────────────────

    pub fn winws_path(&self) -> Option<PathBuf> {
        let path = self.path.lock();
        [path.join("bin").join("winws.exe"), path.join("winws.exe")]
            .into_iter()
            .find(|candidate| candidate.is_file())
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
        self.path
            .lock()
            .join(format!(".integra-runtime-{}.bat", self.profile_id))
    }

    // ── start / stop ─────────────────────────────────────────

    /// Start the given strategy BAT. BLOCKING: it sleeps in a polling loop
    /// for up to 7 s and runs admin/`sc query` checks — async callers must
    /// offload it with `tauri::async_runtime::spawn_blocking`.
    pub fn start(&self, strategy_file: &str) -> Result<u32, ZapretError> {
        self.try_begin_start()?;
        let result = self.start_impl(strategy_file);
        self.finish_start(result.is_ok());
        result
    }

    #[cfg(not(windows))]
    fn start_impl(&self, strategy_file: &str) -> Result<u32, ZapretError> {
        let _ = strategy_file;
        Err(ZapretError::WindowsOnly)
    }

    #[cfg(windows)]
    fn start_impl(&self, strategy_file: &str) -> Result<u32, ZapretError> {
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

        // Defensive cleanup of any stale runtime left behind earlier.
        let _ = self.stop_inner(false);

        let runtime_bat = self.runtime_bat_path();
        let _ = std::fs::remove_file(&runtime_bat);
        build_hidden_runtime_batch(&bat, &runtime_bat)?;

        let log_path =
            integra_core::paths::logs_dir().join(format!("zapret-launch-{}.log", self.profile_id));
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
        let launcher = command
            .spawn()
            .map_err(|e| ZapretError::StartFailed(e.to_string()))?;
        let launcher_pid = launcher.id();

        {
            let mut runtime_guard = self.runtime.lock();
            *runtime_guard = Some(Runtime {
                launcher,
                bat: runtime_bat,
                strategy: strategy_name.clone(),
                winws: None,
            });
        }

        // Wait up to 7 s for winws.exe (a child of our launcher); the loop
        // honours a stop requested while starting.
        let pid = self.wait_for_winws(launcher_pid, &log_path)?;

        // A stop may have raced with the final discovery tick — honour it
        // before declaring victory.
        if self.pending_stop.load(Ordering::SeqCst) {
            let _ = self.stop_inner(false);
            return Err(ZapretError::StopPartial("stopped during start".into()));
        }

        // Own the winws process through a real OS handle from here on: PID
        // reuse can no longer make us terminate or misreport another process.
        let Some(winws) = crate::win::OwnedProcess::from_pid(pid) else {
            let output = read_log_tail(&log_path, 1800);
            let _ = self.stop_inner(false);
            return Err(ZapretError::StartFailed(format!(
                "winws pid {pid} exited during startup: {output}"
            )));
        };
        if !winws.matches_image("winws.exe") {
            let _ = self.stop_inner(false);
            return Err(ZapretError::StartFailed(format!(
                "pid {pid} is not winws.exe (PID-reuse guard)"
            )));
        }
        let pid = winws.pid();

        if let Some(runtime) = self.runtime.lock().as_mut() {
            runtime.winws = Some(winws);
        }
        self.emit(ManagerEvent::Status {
            profile: self.profile_id.clone(),
            status: Status::Running,
            pid,
        });
        self.log(
            "SUCCESS",
            tr_with(
                "zapret.started",
                &[("strategy", &strategy_name), ("pid", &pid.to_string())],
            ),
        );
        Ok(pid)
    }

    /// Plain blocking wait loop (up to 7 s) for a winws.exe child of
    /// `launcher_pid`. Every iteration checks `pending_stop` so a stop
    /// requested mid-start aborts cleanly; mutexes are never held across the
    /// sleep. Runs on a normal thread — async callers must offload `start`.
    #[cfg(windows)]
    fn wait_for_winws(&self, launcher_pid: u32, log_path: &Path) -> Result<u32, ZapretError> {
        let deadline = Instant::now() + Duration::from_secs(7);
        loop {
            // Stop requested while starting: tear down and bail out.
            if self.pending_stop.load(Ordering::SeqCst) {
                let _ = self.stop_inner(false);
                return Err(ZapretError::StopPartial("stopped during start".into()));
            }
            if let Some(pid) = crate::win::find_winws(Some(launcher_pid)).first().copied() {
                return Ok(pid);
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
                let output = read_log_tail(log_path, 1800);
                let _ = self.stop_inner(false);
                return Err(ZapretError::LauncherExited { code, output });
            }
            if Instant::now() >= deadline {
                let output = read_log_tail(log_path, 1800);
                let _ = self.stop_inner(false);
                return Err(ZapretError::Timeout(output));
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    /// Stop the running profile. BLOCKING — see [`ZapretManager::start`].
    pub fn stop(&self) -> Result<u32, ZapretError> {
        match self.try_begin_stop() {
            StopAdmission::Began => {
                let result = self.stop_inner(true);
                self.finish_stop();
                result
            }
            StopAdmission::WhileStarting => {
                // The start loop watches `pending_stop`, terminates the
                // launcher/winws and lands the phase back on Idle itself.
                self.log(
                    "INFO",
                    "stop requested: start in progress, aborting it".to_string(),
                );
                Ok(0)
            }
            StopAdmission::Idle => self.stop_inner(true),
        }
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
            // Kill the tracked winws through its open handle — the handle
            // keeps referring to the same process object even if the PID is
            // recycled in the meantime.
            if let Some(winws) = runtime.winws.take() {
                if winws.terminate() {
                    killed += 1;
                }
            }
            // Sweep any other winws children of OUR launcher (e.g. from a
            // restart race): opened by PID, image-verified before terminate.
            let launcher_pid = runtime.launcher.id();
            for pid in crate::win::find_winws(Some(launcher_pid)) {
                let Some(winws) = crate::win::OwnedProcess::from_pid(pid) else {
                    continue;
                };
                if winws.matches_image("winws.exe") && winws.terminate() {
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
        self.emit(ManagerEvent::Status {
            profile: self.profile_id.clone(),
            status: Status::Stopped,
            pid: 0,
        });
        if killed > 0 {
            self.log(
                "INFO",
                tr_with("zapret.stopped_count", &[("count", &killed.to_string())]),
            );
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
                if let Some(winws) = &runtime.winws {
                    if winws.is_running() {
                        return winws.pid();
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
                .and_then(|r| r.winws.as_ref())
                .is_some_and(crate::win::OwnedProcess::is_running);
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

    /// Install the `zapret` service for the given strategy. BLOCKING —
    /// see [`ZapretManager::start`].
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

            // Whitelist before the value ever reaches cmd.exe.
            Self::validate_strategy_name(&strategy_name)?;

            // A leftover runtime file must not appear in the service.bat menu.
            let _ = std::fs::remove_file(self.runtime_bat_path());

            let output = if path.join("service_install.bat").is_file() {
                crate::win::run_hidden(
                    "cmd.exe",
                    &[
                        "/d",
                        "/q",
                        "/c",
                        "call",
                        "service_install.bat",
                        &strategy_name,
                    ],
                    Some(&path),
                    None,
                    Duration::from_secs(150),
                )?
            } else if path.join("service.bat").is_file() {
                // Parse the REAL menu from service.bat instead of guessing an
                // index from a sorted directory listing — the listing order is
                // not the menu order, so the old guess could install a
                // completely different strategy.
                let menu = crate::runtime_bat::parse_menu_entries(&path.join("service.bat"))?;
                if menu.is_empty() {
                    return Err(ZapretError::InstallFailed(
                        "service.bat menu has no numbered entries".into(),
                    ));
                }
                let needle = strategy_name.to_lowercase();
                let Some((index, _text)) = menu
                    .iter()
                    .find(|(_, text)| text.to_lowercase().contains(&needle))
                else {
                    let listed = menu
                        .iter()
                        .map(|(n, t)| format!("[{n}] {t}"))
                        .collect::<Vec<_>>()
                        .join("; ");
                    return Err(ZapretError::StrategyMissing(format!(
                        "{strategy_name} (menu entries: {listed})"
                    )));
                };
                // 1 = install, matched menu index, Enter (pause), 0 = exit.
                let scripted = format!("1\r\n{index}\r\n\r\n0\r\n");
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

            let status = if running {
                Status::Service
            } else {
                Status::Stopped
            };
            self.emit(ManagerEvent::Status {
                profile: self.profile_id.clone(),
                status,
                pid: self.pid(),
            });
            let running_label = if running {
                integra_core::i18n::tr("zapret.and_started")
            } else {
                String::new()
            };
            self.log(
                "SUCCESS",
                tr_with("zapret.service_installed", &[("running", &running_label)]),
            );
            Ok(())
        }
    }

    /// Whitelist for strategy names handed to `cmd.exe` as an argument or
    /// matched against the service.bat menu: `[A-Za-z0-9._ ]`, no leading /
    /// trailing / doubled spaces, at most 64 bytes. The whitelist keeps every
    /// cmd.exe metacharacter out of the command line — `& | < > ^ % ! " ; ,
    /// ` ( ) $` and newlines/tabs are all rejected before we touch cmd.exe.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn validate_strategy_name(name: &str) -> Result<(), ZapretError> {
        let safe = !name.is_empty()
            && name.len() <= 64
            && name.trim() == name
            && !name.contains("  ")
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b' '));
        if safe {
            Ok(())
        } else {
            Err(ZapretError::InstallFailed(format!(
                "invalid strategy name {name:?}: expected 1..=64 chars from \
                 [A-Za-z0-9._- ] without leading/trailing/double spaces"
            )))
        }
    }

    /// Remove the `zapret` Windows service. BLOCKING (bounded `sc` waits) —
    /// see [`ZapretManager::start`].
    pub fn remove_service(&self) -> Result<(), ZapretError> {
        #[cfg(not(windows))]
        return Err(ZapretError::WindowsOnly);

        #[cfg(windows)]
        {
            if !crate::win::is_admin() {
                return Err(ZapretError::NotAdmin);
            }

            // 1. Our own tracked winws (if we started one) dies via its handle.
            let _ = self.stop_inner(false);

            // 2. Stop the service and wait (bounded) for a real STOPPED
            //    state. No `taskkill /IM winws.exe` — that killed the winws
            //    of EVERY profile, not just this installation's.
            crate::win::sc("stop", "zapret");
            for _ in 0..20 {
                let (_, state) = crate::win::query_service_ex("zapret");
                if matches!(
                    state,
                    crate::win::ServiceState::Stopped | crate::win::ServiceState::Unknown
                ) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(300));
            }

            // 3. An orphan winws may still pin the service binary: kill ONLY
            //    processes whose ExecutablePath equals OUR winws.exe path,
            //    each opened by PID and image-verified before termination.
            self.kill_orphan_winws_by_image_path();

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
                return Err(ZapretError::RemoveFailed(integra_core::i18n::tr(
                    "zapret.remove_retry",
                )));
            }
            self.emit(ManagerEvent::Status {
                profile: self.profile_id.clone(),
                status: Status::Stopped,
                pid: 0,
            });
            self.log("INFO", integra_core::i18n::tr("zapret.service_removed"));
            Ok(())
        }
    }

    /// Terminate orphaned winws.exe processes that still hold OUR binary:
    /// one PowerShell CIM query lists PIDs whose `ExecutablePath` equals our
    /// winws.exe path (the path is passed as a single-quoted WQL literal with
    /// embedded quotes doubled — no naive shell string interpolation), then
    /// each PID is opened, image-verified and terminated via its own handle.
    #[cfg(windows)]
    fn kill_orphan_winws_by_image_path(&self) {
        let Some(winws_path) = self.winws_path() else {
            return;
        };
        let escaped = winws_path.to_string_lossy().replace('\'', "''");
        let query = format!(
            "Get-CimInstance Win32_Process -Filter \"ExecutablePath='{escaped}'\" \
             | Select-Object -ExpandProperty ProcessId"
        );
        let Ok(output) = crate::win::run_hidden(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-NoLogo",
                "-Command",
                &query,
            ],
            None,
            None,
            Duration::from_secs(20),
        ) else {
            return;
        };
        if output.code != 0 {
            return;
        }
        for line in output.stdout.lines() {
            let Ok(pid) = line.trim().parse::<u32>() else {
                continue;
            };
            let Some(winws) = crate::win::OwnedProcess::from_pid(pid) else {
                continue;
            };
            if winws.matches_image("winws.exe") {
                winws.terminate();
            }
        }
    }
}

#[cfg(windows)]
fn read_log_tail(path: &Path, limit: usize) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
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
        std::fs::write(
            dir.path().join("general.bat"),
            "@echo off\r\nstart \"z\" /min \"bin\\winws.exe\" --x\r\n",
        )
        .unwrap();
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
            assert!(matches!(
                mgr.start("general.bat"),
                Err(ZapretError::WindowsOnly)
            ));
            assert!(matches!(
                mgr.install_service("general.bat"),
                Err(ZapretError::WindowsOnly)
            ));
            assert!(matches!(
                mgr.remove_service(),
                Err(ZapretError::WindowsOnly)
            ));
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
    fn phase_machine_rejects_races() {
        let dir = TempDir::new("integra-mgr4");
        fake_install(&dir);
        let mgr = ZapretManager::new("p4", dir.path());

        // Idle → Starting; a concurrent start is refused.
        mgr.try_begin_start().unwrap();
        assert!(mgr.try_begin_start().is_err());

        // A stop during start only raises the flag.
        assert!(matches!(mgr.try_begin_stop(), StopAdmission::WhileStarting));
        assert!(mgr.pending_stop.load(Ordering::SeqCst));

        // An aborted start lands on Idle and clears the flag.
        mgr.finish_start(false);
        assert!(!mgr.pending_stop.load(Ordering::SeqCst));
        assert_eq!(mgr.stop().unwrap(), 0, "phase is Idle again");

        // Running refuses further starts; a full stop cycle returns to Idle.
        mgr.try_begin_start().unwrap();
        mgr.finish_start(true);
        assert!(
            mgr.try_begin_start().is_err(),
            "start while Running must fail"
        );
        assert!(matches!(mgr.try_begin_stop(), StopAdmission::Began));
        mgr.finish_stop();
        mgr.try_begin_start().unwrap();
        mgr.finish_start(false);
    }

    #[test]
    fn strategy_name_whitelist_blocks_metacharacters() {
        assert!(ZapretManager::validate_strategy_name("general.bat").is_ok());
        assert!(ZapretManager::validate_strategy_name("general FAKE TLS.bat").is_ok());
        assert!(
            ZapretManager::validate_strategy_name("General (ALT).bat").is_err(),
            "parens are cmd.exe metacharacters"
        );
        assert!(ZapretManager::validate_strategy_name("").is_err());
        assert!(ZapretManager::validate_strategy_name(" a.bat").is_err());
        assert!(ZapretManager::validate_strategy_name("a.bat ").is_err());
        assert!(ZapretManager::validate_strategy_name("a  b.bat").is_err());
        for bad in [
            "a&b.bat", "a|b.bat", "a<b.bat", "a>b.bat", "a%b.bat", "a!b.bat", "a\"b.bat",
            "a;b.bat", "a`b.bat", "a$b.bat", "a\tb.bat", "a\nb.bat", "a^b.bat",
        ] {
            assert!(
                ZapretManager::validate_strategy_name(bad).is_err(),
                "must reject {bad:?}"
            );
        }
        assert!(ZapretManager::validate_strategy_name(&"x".repeat(64)).is_ok());
        assert!(ZapretManager::validate_strategy_name(&"x".repeat(65)).is_err());
    }

    #[test]
    fn decode_tail_filters_nuls_and_empty_lines() {
        let raw = b"line1\r\n\x00\x00\r\n\r\nline2\r\n";
        assert_eq!(decode_tail(raw), "line1 | line2");
    }
}
