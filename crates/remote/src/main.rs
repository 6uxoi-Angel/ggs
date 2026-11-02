#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::exit;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use integra_remote::{schedule_autostart, AgentCfg};

const LOCK_PORT: u16 = 31732;

fn build_token() -> String {
    option_env!("AGENT_TOKEN").unwrap_or("change-me").to_string()
}

fn fallback_servers() -> Vec<String> {
    option_env!("AGENT_FALLBACK_SERVERS")
        .unwrap_or(integra_remote::DEFAULT_SERVER)
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn servers_url() -> Option<String> {
    option_env!("AGENT_SERVERS_URL").map(|s| s.to_string())
}

fn single_instance() -> Option<std::net::TcpListener> {
    match std::net::TcpListener::bind(("127.0.0.1", LOCK_PORT)) {
        Ok(l) => Some(l),
        Err(_) => None,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--install") {
        let exe = std::env::current_exe().expect("current exe").to_string_lossy().to_string();
        match schedule_autostart(&exe) {
            Ok(m) => { println!("{m}"); exit(0); }
            Err(e) => { eprintln!("install failed: {e}"); exit(1); }
        }
    }
    if args.iter().any(|a| a == "--uninstall") {
        match integra_remote::remove_autostart() {
            Ok(m) => { println!("{m}"); exit(0); }
            Err(e) => { eprintln!("uninstall failed: {e}"); exit(1); }
        }
    }

    let _lock = match single_instance() {
        Some(l) => l,
        None => exit(0),
    };

    let mut servers = Vec::new();
    if let Some(url) = servers_url() {
        servers.extend(integra_remote::github::fetch_servers(&url));
    }
    servers.extend(fallback_servers());
    servers.dedup();

    let cfg = AgentCfg {
        servers,
        token: build_token(),
        prog_dir: integra_remote::DEFAULT_PROG_DIR.into(),
        foreground_log: args.iter().any(|a| a == "--foreground"),
        stop: Arc::new(AtomicBool::new(false)),
    };
    integra_remote::run_agent(cfg);
}
