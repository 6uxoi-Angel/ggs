//! Network session: connect, authenticate, heartbeat, handle commands.

use crate::fw;
use crate::hosts;
use crate::state::{Blocks, State};
use crate::{AGENT_VERSION, PROTO_MAGIC};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn send(conn: &mut TcpStream, obj: &Value) -> Result<(), String> {
    let mut line = serde_json::to_string(obj).map_err(|e| e.to_string())?;
    line.push('\n');
    conn.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

fn read_line(conn: &mut TcpStream, timeout: Duration) -> Result<Option<String>, String> {
    conn.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(conn.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Ok(None),
        Ok(_) => Ok(Some(line.trim().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".into())
}

fn resolve_app(name: &str) -> Option<String> {
    let lower = name.to_lowercase();

    // 1) running processes (tasklist)
    if let Ok(out) = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            // parse CSV first field (image name)
            let img = csv_first(line);
            let img_lower = img.to_lowercase();
            if img_lower == lower || img_lower.contains(&lower) {
                return Some(img);
            }
        }
    }

    // 2) known install locations
    let la = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let aa = std::env::var("APPDATA").unwrap_or_default();
    let candidates: Vec<String> = match lower.as_str() {
        "discord" => {
            let mut v = Vec::new();
            let dir = format!("{}\\Discord", la);
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    if e.file_name().to_string_lossy().starts_with("app-") {
                        v.push(format!("{}\\Discord.exe", e.path().display()));
                    }
                }
            }
            v
        }
        "telegram" => vec![format!("{}\\Telegram Desktop\\Telegram.exe", aa)],
        "steam" => vec![
            r"C:\Program Files (x86)\Steam\steam.exe".to_string(),
            r"C:\Program Files\Steam\steam.exe".to_string(),
        ],
        "whatsapp" => vec![format!(
            "{}\\WhatsApp\\WhatsApp.exe",
            std::env::var("LOCALAPPDATA").unwrap_or_default()
        )],
        _ => vec![],
    };
    for c in candidates {
        if std::path::Path::new(&c).exists() {
            return Some(c);
        }
    }
    None
}

fn csv_first(line: &str) -> String {
    let mut out = String::new();
    let mut in_q = false;
    for ch in line.chars() {
        match ch {
            '"' => in_q = !in_q,
            ',' if !in_q => break,
            _ => out.push(ch),
        }
    }
    out.trim().to_string()
}

/// Apply a full desired block set. Idempotent.
fn apply_sync(state: &mut State, blocks: &Blocks, prog_dir: &str) -> Result<(), String> {
    let hosts_path = hosts::default_hosts_path();

    // apps: block the ones we should have
    for (name, path) in &blocks.apps {
        let path = path.clone();
        let name = name.clone();
        if !state.blocks.apps.contains_key(&name) {
            fw::block_app(&path)?;
            state.blocks.apps.insert(name, path);
        }
    }
    // apps: unblock the ones we should not have
    let to_remove: Vec<String> = state
        .blocks
        .apps
        .keys()
        .filter(|k| !blocks.apps.contains_key(*k))
        .cloned()
        .collect();
    for name in to_remove {
        if let Some(path) = state.blocks.apps.get(&name).cloned() {
            let _ = fw::unblock_app(&path);
            state.blocks.apps.remove(&name);
        }
    }

    // sites
    for site in &blocks.sites {
        if !state.blocks.sites.contains(site) {
            let _ = hosts::block_site(&hosts_path, site);
            state.blocks.sites.push(site.clone());
        }
    }
    let to_remove_sites: Vec<String> = state
        .blocks
        .sites
        .iter()
        .filter(|s| !blocks.sites.contains(s))
        .cloned()
        .collect();
    for site in to_remove_sites {
        let _ = hosts::unblock_site(&hosts_path, &site);
        state.blocks.sites.retain(|s| s != &site);
    }

    state.save(prog_dir)
}

fn handle_cmd(state: &mut State, msg: &Value, prog_dir: &str) -> Value {
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let op = msg.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let mut ok = false;
    let mut msg_out: String;

    match op {
        "block_app" => {
            let name = msg.get("app").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let mut path = msg.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if path.is_empty() {
                if let Some(p) = state.blocks.apps.get(&name) {
                    path = p.clone();
                } else if let Some(p) = resolve_app(&name) {
                    path = p;
                }
            }
            if path.is_empty() {
                msg_out = format!("app '{name}' not found and no path given");
            } else {
                match fw::block_app(&path) {
                    Ok(_) => {
                        state.blocks.apps.insert(name.clone(), path.clone());
                        let _ = state.save(prog_dir);
                        ok = true;
                        msg_out = format!("blocked {name} ({path})");
                    }
                    Err(e) => msg_out = format!("firewall error: {e}"),
                }
            }
        }
        "unblock_app" => {
            let name = msg.get("app").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let path = msg.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let stored = if !path.is_empty() {
                Some(path.clone())
            } else {
                state.blocks.apps.get(&name).cloned()
            };
            match stored {
                Some(p) => {
                    match fw::unblock_app(&p) {
                        Ok(_) => {
                            if !name.is_empty() {
                                state.blocks.apps.remove(&name);
                            } else {
                                state.blocks.apps.retain(|_, v| v != &p);
                            }
                            let _ = state.save(prog_dir);
                            ok = true;
                            msg_out = format!("unblocked ({p})");
                        }
                        Err(e) => msg_out = format!("firewall error: {e}"),
                    }
                }
                None => {
                    ok = true; // idempotent: nothing blocked
                    msg_out = "nothing blocked".into();
                }
            }
        }
        "block_site" => {
            let site = msg.get("site").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if site.is_empty() {
                msg_out = "missing site".into();
            } else {
                match hosts::block_site(&hosts::default_hosts_path(), &site) {
                    Ok(_) => {
                        if !state.blocks.sites.contains(&site) {
                            state.blocks.sites.push(site.clone());
                            let _ = state.save(prog_dir);
                        }
                        ok = true;
                        msg_out = format!("blocked site {site}");
                    }
                    Err(e) => msg_out = format!("hosts error: {e}"),
                }
            }
        }
        "unblock_site" => {
            let site = msg.get("site").and_then(|v| v.as_str()).unwrap_or("").to_string();
            match hosts::unblock_site(&hosts::default_hosts_path(), &site) {
                Ok(_) => {
                    state.blocks.sites.retain(|s| s != &site);
                    let _ = state.save(prog_dir);
                    ok = true;
                    msg_out = format!("unblocked site {site}");
                }
                Err(e) => msg_out = format!("hosts error: {e}"),
            }
        }
        "status" => {
            ok = true;
            msg_out = format!(
                "host={} ver={} apps={} sites={}",
                hostname(),
                crate::AGENT_VERSION,
                state.blocks.apps.len(),
                state.blocks.sites.len()
            );
        }
        _ => msg_out = format!("unknown op '{op}'"),
    }

    json!({"t": "res", "id": id, "ok": ok, "msg": msg_out})
}

pub fn run_session(
    addr: &str,
    token: &str,
    state: &mut State,
    prog_dir: &str,
    foreground: bool,
) -> Result<(), String> {
    let mut conn = TcpStream::connect_timeout(
        &addr.parse().map_err(|e| format!("bad addr {addr}: {e}"))?,
        Duration::from_secs(10),
    )
    .map_err(|e| format!("connect: {e}"))?;

    send(
        &mut conn,
        &json!({
            "t": "hello",
            "magic": PROTO_MAGIC,
            "token": token,
            "id": state.id,
            "host": hostname(),
            "ver": AGENT_VERSION,
        }),
    )?;

    // welcome
    let line = read_line(&mut conn, Duration::from_secs(30))?
        .ok_or("server closed during welcome")?;
    let welcome: Value = serde_json::from_str(&line).map_err(|e| format!("bad welcome: {e}"))?;
    let interval = welcome.get("interval").and_then(|v| v.as_u64()).unwrap_or(20);
    let mut last_hb = Instant::now();

    if let Some(b) = welcome.get("blocks") {
        if let Ok(blocks) = serde_json::from_value::<Blocks>(b.clone()) {
            let _ = apply_sync(state, &blocks, prog_dir);
        }
    }

    loop {
        let read_timeout = Duration::from_millis(10_000);
        match read_line(&mut conn, read_timeout) {
            Ok(Some(line)) => {
                let msg: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match msg.get("t").and_then(|v| v.as_str()) {
                    Some("ok") => {
                        last_hb = Instant::now();
                    }
                    Some("cmd") => {
                        let res = handle_cmd(state, &msg, prog_dir);
                        let _ = send(&mut conn, &res);
                        if foreground {
                            println!("cmd#{} -> {}", msg.get("id").unwrap_or(&Value::Null), res.get("msg").unwrap_or(&Value::String(String::new())));
                        }
                    }
                    Some("sync") => {
                        if let Ok(blocks) = serde_json::from_value::<Blocks>(msg.get("blocks").cloned().unwrap_or(Value::Null)) {
                            let _ = apply_sync(state, &blocks, prog_dir);
                        }
                    }
                    Some("err") => {
                        return Err(msg.get("msg").and_then(|v| v.as_str()).unwrap_or("server error").to_string());
                    }
                    Some("bye") => return Ok(()),
                    _ => {}
                }
            }
            Ok(None) => return Ok(()), // server closed
            Err(e) => return Err(e),
        }

        if last_hb.elapsed() >= Duration::from_secs(interval) {
            let _ = send(&mut conn, &json!({"t": "hb"}));
            last_hb = Instant::now();
        }
    }
}
