//! TG WS Proxy manager: locate the project, prepare a Python venv, run the
//! headless proxy as a hidden child process, tail its logs and build the
//! `tg://proxy` link for Telegram Desktop.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use crate::CommandError;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

pub struct TgProxyInner {
    pub child: Option<Child>,
    pub logs: VecDeque<String>,
    pub setup_running: bool,
    /// Real port the proxy reported via `LISTENING <port>` while running.
    pub port: Option<u16>,
}

impl Default for TgProxyInner {
    fn default() -> Self {
        Self {
            child: None,
            logs: VecDeque::with_capacity(600),
            setup_running: false,
            port: None,
        }
    }
}

impl TgProxyInner {
    pub fn push_log(&mut self, line: String) {
        if self.logs.len() >= 600 {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    pub fn tail(&self, n: usize) -> Vec<String> {
        let n = n.min(self.logs.len());
        self.logs
            .iter()
            .skip(self.logs.len() - n)
            .cloned()
            .collect()
    }

    pub fn is_running(&mut self) -> bool {
        self.child
            .as_mut()
            .map(|c| c.try_wait().map(|s| s.is_none()).unwrap_or(false))
            .unwrap_or(false)
    }
}

pub type TgProxy = Mutex<TgProxyInner>;

/// Resolve the TG WS Proxy project directory.
pub fn find_dir(cfg_path: &str) -> Option<PathBuf> {
    let cfg = cfg_path.trim();
    if !cfg.is_empty() {
        let p = PathBuf::from(cfg);
        if p.join("proxy").join("tg_ws_proxy.py").exists() {
            return Some(p);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(home) = std::env::var("USERPROFILE") {
        candidates.push(PathBuf::from(&home).join("Desktop/ggs/tg-ws-proxy-main"));
        candidates.push(PathBuf::from(&home).join("Desktop/tg-ws-proxy-main"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("tg-ws-proxy-main"));
            candidates.push(dir.join("..").join("tg-ws-proxy-main"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("tg-ws-proxy-main"));
    }
    candidates
        .into_iter()
        .find(|p| p.join("proxy").join("tg_ws_proxy.py").exists())
}

pub fn python_bin(dir: &std::path::Path) -> PathBuf {
    let venv = dir.join(".venv/Scripts/python.exe");
    if venv.exists() {
        venv
    } else {
        PathBuf::from("python")
    }
}

pub fn read_version(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("proxy/__init__.py"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.contains("__version__"))
                .and_then(|l| l.split('=').nth(1))
                .map(|v| v.trim().trim_matches(|c| c == '\'' || c == '"').to_string())
        })
        .unwrap_or_default()
}

/// True when the external `tg_ws_proxy.py` supports the Integra handshake
/// (secret via the `TGPROXY_SECRET` env var, ephemeral port + the
/// `LISTENING <port>` stdout line). Detected by scanning the script for the
/// protocol markers; older builds fall back to the compatibility launcher.
fn supports_handshake(dir: &std::path::Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("proxy/tg_ws_proxy.py")) else {
        return false;
    };
    text.contains("TGPROXY_SECRET") && text.contains("LISTENING")
}

/// In-process launcher for legacy external builds: injects `--secret` and
/// `--port` from the environment into the module's argv INSIDE the python
/// process, so the secret never appears in any OS command line.
const LAUNCHER_NAME: &str = "integra_proxy_launch.py";

const LAUNCHER_SRC: &str = r#"import os
import runpy
import sys

args = list(sys.argv[1:])
secret = os.environ.get("TGPROXY_SECRET", "")
port = os.environ.get("TGPROXY_PORT", "")
if secret and "--secret" not in args:
    args += ["--secret", secret]
if port:
    if "--port" in args:
        i = args.index("--port")
        if i + 1 < len(args):
            args[i + 1] = port
    else:
        args += ["--port", port]
sys.argv = ["tg_ws_proxy.py"] + args
runpy.run_module("proxy.tg_ws_proxy", run_name="__main__")
"#;

fn ensure_launcher(dir: &std::path::Path) -> Result<PathBuf, CommandError> {
    let path = dir.join(LAUNCHER_NAME);
    match std::fs::read_to_string(&path) {
        Ok(existing) if existing == LAUNCHER_SRC => {}
        _ => {
            integra_core::fsutil::atomic_write(&path, LAUNCHER_SRC.as_bytes())
                .map_err(|e| CommandError::raw(format!("launcher write failed: {e}")))?;
        }
    }
    Ok(path)
}

/// How long to wait for the python proxy to report its real (ephemeral)
/// listening port via the `LISTENING <port>` stdout line.
const LISTEN_TIMEOUT: Duration = Duration::from_secs(15);

pub fn spawn_proxy(
    shared: Arc<TgProxy>,
    dir: &std::path::Path,
    port: u16,
    secret: &str,
    log_dir: &std::path::Path,
) -> Result<u32, CommandError> {
    let python = python_bin(dir);
    std::fs::create_dir_all(log_dir).map_err(|e| CommandError::raw(e.to_string()))?;
    let log_file = log_dir.join("tgproxy.log");

    {
        // Refuse a double start before spawning anything (the old code
        // spawned first and killed the doomed child afterwards).
        let mut guard = shared.lock();
        if guard.is_running() {
            return Err(CommandError::key("tgproxy.already_running"));
        }
    }

    // New external builds support the Integra handshake: the secret arrives
    // via the TGPROXY_SECRET env var, the proxy binds port 0 and reports the
    // real (OS-picked) port through the `LISTENING <port>` stdout line. Older
    // builds only accept --secret/--port on argv; for them a generated
    // in-process launcher injects the secret from the environment (so it
    // never appears in any OS command line) and we poll the configured port.
    let handshake = supports_handshake(dir);
    let mut cmd = Command::new(&python);
    if handshake {
        cmd.arg("-m")
            .arg("proxy.tg_ws_proxy")
            // Port 0 → the OS picks a free port; the real one arrives via the
            // `LISTENING <port>` stdout line (closes the port-selection race).
            .arg("--port")
            .arg("0");
    } else {
        let launcher = ensure_launcher(dir)?;
        cmd.arg(&launcher).arg("--port").arg(port.to_string());
    }
    cmd.arg("--log-file")
        .arg(&log_file)
        .arg("--log-max-mb")
        .arg("2")
        .arg("--log-backups")
        .arg("1")
        .arg("--buf-kb")
        .arg("256")
        .arg("--pool-size")
        .arg("4")
        .env("TGPROXY_SECRET", secret)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !handshake {
        // Legacy launcher reads the configured port from the environment too.
        cmd.env("TGPROXY_PORT", port.to_string());
    }
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| {
        CommandError::raw(format!(
            "не удалось запустить Python ({e}); установите зависимости кнопкой ниже"
        ))
    })?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Dedicated one-shot channel for the `LISTENING <port>` line. The stdout
    // reader keeps draining the pipe the whole time (so the child can never
    // deadlock on a full pipe) while we wait on the channel with a bound.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<u16>();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Some(stream) = stdout {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stream);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if let Some(port) = line.trim().strip_prefix("LISTENING ") {
                    if let Ok(port) = port.trim().parse::<u16>() {
                        let _ = ready_tx.send(port);
                    }
                }
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    if let Some(stream) = stderr {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stream);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        });
    }
    {
        let shared = shared.clone();
        std::thread::spawn(move || {
            while let Ok(line) = rx.recv() {
                shared.lock().push_log(line);
            }
        });
    }

    // Wait (bounded) for the proxy to become reachable. In handshake mode a
    // disconnected channel means the child's stdout is gone — fail fast
    // instead of burning the full timeout.
    let bound_port = if handshake {
        match ready_rx.recv_timeout(LISTEN_TIMEOUT) {
            Ok(port) => port,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = Command::new("taskkill")
                    .arg("/PID")
                    .arg(pid.to_string())
                    .arg("/T")
                    .arg("/F")
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
                let _ = child.kill();
                let _ = child.wait();
                return Err(CommandError::raw(format!(
                    "прокси не сообщил порт за {LISTEN_TIMEOUT:?}; ожидалась строка \"LISTENING <порт>\" в stdout"
                )));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CommandError::raw(
                    "поток вывода прокси закрылся до старта (см. журнал tgproxy)",
                ));
            }
        }
    } else {
        // Legacy external build: poll the configured port with TCP connects
        // and bail out early if the child already exited (bad deps, argparse
        // error, ...).
        let deadline = std::time::Instant::now() + LISTEN_TIMEOUT;
        loop {
            if child.try_wait().map(|s| s.is_some()).unwrap_or(true) {
                let _ = child.wait();
                return Err(CommandError::raw(
                    "прокси завершился до открытия порта (см. журнал tgproxy)",
                ));
            }
            if port_open(port) {
                break port;
            }
            if std::time::Instant::now() >= deadline {
                let _ = Command::new("taskkill")
                    .arg("/PID")
                    .arg(pid.to_string())
                    .arg("/T")
                    .arg("/F")
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
                let _ = child.kill();
                let _ = child.wait();
                return Err(CommandError::raw(format!(
                    "прокси не открыл порт {port} за {LISTEN_TIMEOUT:?}"
                )));
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    };

    {
        let mut guard = shared.lock();
        guard.child = Some(child);
        guard.port = Some(bound_port);
        guard.push_log(format!(
            "[integra] прокси запущен (PID {pid}, порт {bound_port})"
        ));
        if !handshake {
            guard.push_log(
                "[integra] внешний tg-ws-proxy без поддержки TGPROXY_SECRET/LISTENING — режим совместимости; рекомендуется обновить внешний проект".into(),
            );
        }
    }

    Ok(pid)
}

pub fn stop_proxy(shared: Arc<TgProxy>) {
    let mut guard = shared.lock();
    if let Some(child) = guard.child.as_mut() {
        let pid = child.id();
        let _ = Command::new("taskkill")
            .arg("/PID")
            .arg(pid.to_string())
            .arg("/T")
            .arg("/F")
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        let _ = child.kill();
        let _ = child.wait();
        guard.child = None;
        guard.port = None;
        guard.push_log("[integra] прокси остановлен".into());
    }
}

pub fn setup_venv(shared: Arc<TgProxy>, dir: &std::path::Path) -> Result<(), CommandError> {
    {
        // Atomically claim the setup slot: a concurrent second caller gets a
        // clear error instead of racing us on a half-built venv.
        let mut guard = shared.lock();
        if guard.setup_running {
            return Err(CommandError::key("tgproxy.setup_running"));
        }
        guard.setup_running = true;
    }
    // This drop guard guarantees the flag is cleared on EVERY exit path
    // (early returns, errors and panics alike).
    struct SetupGuard(Arc<TgProxy>);
    impl std::ops::Drop for SetupGuard {
        fn drop(&mut self) {
            self.0.lock().setup_running = false;
        }
    }
    let _setup_guard = SetupGuard(shared.clone());

    shared
        .lock()
        .push_log("[integra] подготовка окружения Python…".into());
    let venv = dir.join(".venv");
    let sys_python = PathBuf::from("python");

    if !venv.join("Scripts").join("python.exe").exists() {
        let out = Command::new(&sys_python)
            .args(["-m", "venv"])
            .arg(&venv)
            .current_dir(dir)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| CommandError::raw(format!("venv create failed: {e}")))?;
        if !out.status.success() {
            return Err(CommandError::raw(format!(
                "не удалось создать venv: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        shared.lock().push_log("[integra] venv создан".into());
    }

    let pip = venv.join("Scripts").join("pip.exe");
    let install = Command::new(&pip)
        .args([
            "install",
            "--disable-pip-version-check",
            "--quiet",
            "httpx[http2]==0.28.1",
            "cryptography",
            "certifi",
        ])
        .current_dir(dir)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| CommandError::raw(format!("pip failed: {e}")))?;
    if !install.status.success() {
        return Err(CommandError::raw(format!(
            "установка зависимостей не удалась: {}",
            String::from_utf8_lossy(&install.stderr)
        )));
    }
    shared
        .lock()
        .push_log("[integra] зависимости установлены".into());
    Ok(())
}

pub fn port_open(port: u16) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}
