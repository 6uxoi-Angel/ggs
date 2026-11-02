//! Linux collector: parses /proc/net/{tcp,tcp6,udp,udp6} and maps socket
//! inodes to PIDs. Used for development/testing of the tracker in sandboxes;
//! production Windows builds use [`crate::windows_tables`].

use std::collections::HashMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;

use crate::collector::Collector;
use crate::{Connection, ProcessInfo, Protocol, TcpState};

#[derive(Default)]
pub struct ProcFsCollector;

impl Collector for ProcFsCollector {
    fn snapshot(&self) -> Vec<Connection> {
        let inode_to_pid = inode_pid_map();
        let mut out = Vec::new();
        for (file, proto) in [
            ("/proc/net/tcp", Protocol::Tcp4),
            ("/proc/net/tcp6", Protocol::Tcp6),
            ("/proc/net/udp", Protocol::Udp4),
            ("/proc/net/udp6", Protocol::Udp6),
        ] {
            out.extend(parse_net_file(Path::new(file), proto, &inode_to_pid));
        }
        out
    }

    fn processes(&self) -> Vec<ProcessInfo> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir("/proc") else {
            return out;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(pid) = name.parse::<u32>() {
                let comm = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
                let exe = fs::read_link(format!("/proc/{pid}/exe"))
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
                out.push(ProcessInfo {
                    pid,
                    name: comm.trim().to_string(),
                    exe,
                });
            }
        }
        out
    }
}

fn parse_net_file(
    path: &Path,
    proto: Protocol,
    inode_to_pid: &HashMap<u64, u32>,
) -> Vec<Connection> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let is_tcp = matches!(proto, Protocol::Tcp4 | Protocol::Tcp6);
    let is_v6 = matches!(proto, Protocol::Tcp6 | Protocol::Udp6);
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 {
                return None;
            }
            let local = parse_addr(f[1], is_v6)?;
            let remote = parse_addr(f[2], is_v6)?;
            let state = if is_tcp {
                TcpState::from_raw(u32::from_str_radix(f[3], 16).ok()?)
            } else {
                None
            };
            let inode = f[9].parse::<u64>().ok()?;
            let pid = inode_to_pid.get(&inode).copied().unwrap_or(0);
            let remote_opt = if remote.ip().is_unspecified() && remote.port() == 0 {
                None
            } else {
                Some(remote)
            };
            Some(Connection {
                proto,
                local,
                remote: remote_opt,
                state,
                pid,
            })
        })
        .collect()
}

fn parse_addr(text: &str, v6: bool) -> Option<SocketAddr> {
    let (addr_part, port_part) = text.rsplit_once(':')?;
    let port = u16::from_str_radix(port_part, 16).ok()?;
    let ip: IpAddr = if v6 {
        // 32 hex chars, four little-endian 32-bit words.
        if addr_part.len() != 32 {
            return None;
        }
        let mut octets = [0u8; 16];
        for word in 0..4 {
            let raw = u32::from_str_radix(&addr_part[word * 8..word * 8 + 8], 16).ok()?;
            octets[word * 4..word * 4 + 4].copy_from_slice(&raw.to_le_bytes());
        }
        Ipv6Addr::from(octets).into()
    } else {
        let raw = u32::from_str_radix(addr_part, 16).ok()?;
        Ipv4Addr::from(raw.to_le_bytes()).into()
    };
    Some(SocketAddr::new(ip, port))
}

fn inode_pid_map() -> HashMap<u64, u32> {
    let mut map = HashMap::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return map;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let fd_dir = format!("/proc/{pid}/fd");
        let Ok(fds) = fs::read_dir(&fd_dir) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(link) = fs::read_link(fd.path()) {
                let link = link.to_string_lossy();
                if let Some(inode) = link
                    .strip_prefix("socket:[")
                    .and_then(|s| s.strip_suffix(']'))
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    map.insert(inode, pid);
                }
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_addresses() {
        // 127.0.0.1:80 in /proc hex (little-endian words)
        let addr = parse_addr("0100007F:0050", false).unwrap();
        assert_eq!(addr.to_string(), "127.0.0.1:80");
        let v6 = parse_addr("00000000000000000000000001000000:01BB", true).unwrap();
        assert_eq!(v6.port(), 443);
        assert_eq!(v6.ip(), IpAddr::V6(Ipv6Addr::LOCALHOST));
    }

    #[test]
    fn live_snapshot_runs() {
        let collector = ProcFsCollector::default();
        // Just ensure parsing doesn't panic on the real /proc of the sandbox.
        let snap = collector.snapshot();
        let procs = collector.processes();
        assert!(!procs.is_empty());
        for conn in snap.iter().take(5) {
            assert_ne!(conn.local.port(), 0, "parsed ports look sane");
        }
    }
}
