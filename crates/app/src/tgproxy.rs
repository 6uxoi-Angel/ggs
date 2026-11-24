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
}

impl Default for TgProxyInner {
    fn default() -> Self {
        Self {
            child: None,
            logs: VecDeque::with_capacity(600),
            setup_running: false,
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
        self.logs.iter().skip(self.logs.len() - n).cloned().collect()
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

    let mut cmd = Command::new(&python);
    cmd.arg("-m")
        .arg("proxy.tg_ws_proxy")
        .arg("--port")
        .arg(port.to_string())
        .arg("--secret")
        .arg(secret)
        .arg("--log-file")
        .arg(&log_file)
        .arg("--log-max-mb")
        .arg("2")
        .arg("--log-backups")
        .arg("1")
        .arg("--buf-kb")
        .arg("256")
        .arg("--pool-size")
        .arg("4")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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

    {
        let mut guard = shared.lock();
        if guard.is_running() {
            let _ = child.kill();
            return Err(CommandError::key("tgproxy.already_running"));
        }
        guard.child = Some(child);
        guard.push_log(format!("[integra] прокси запущен (PID {pid}, порт {port})"));
    }

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Some(stream) = stdout {
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
        guard.push_log("[integra] прокси остановлен".into());
    }
}

pub fn setup_venv(shared: Arc<TgProxy>, dir: &std::path::Path) -> Result<(), CommandError> {
    {
        let guard = shared.lock();
        if guard.setup_running {
            return Err(CommandError::key("tgproxy.setup_running"));
        }
    }
    shared.lock().push_log("[integra] подготовка окружения Python…".into());
    let venv = dir.join(".venv");
    let sys_python = PathBuf::from("python");

    if !venv.join("Scripts").join("python.exe").exists() {
        let out = Command::new(&sys_python)
            .args(["-m", "venv"])
            .arg(&venv)
            .current_dir(dir)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| {
                shared.lock().setup_running = false;
                CommandError::raw(format!("venv create failed: {e}"))
            })?;
        if !out.status.success() {
            shared.lock().setup_running = false;
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
        .map_err(|e| {
            shared.lock().setup_running = false;
            CommandError::raw(format!("pip failed: {e}"))
        })?;
    if !install.status.success() {
        shared.lock().setup_running = false;
        return Err(CommandError::raw(format!(
            "установка зависимостей не удалась: {}",
            String::from_utf8_lossy(&install.stderr)
        )));
    }
    {
        let mut guard = shared.lock();
        guard.setup_running = false;
        guard.push_log("[integra] зависимости установлены".into());
    }
    Ok(())
}

pub fn port_open(port: u16) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}
