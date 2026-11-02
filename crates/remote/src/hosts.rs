//! Site blocking via the hosts file + DNS cache flush.

use std::path::Path;
use std::process::Command;

pub const MARKER: &str = "# netguard-block";

pub fn default_hosts_path() -> String {
    "C:\\Windows\\System32\\drivers\\etc\\hosts".to_string()
}

fn line_domain(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.contains(MARKER) {
        return None;
    }
    let mut it = line.split_whitespace();
    let _ip = it.next()?;
    let domain = it.next()?;
    Some(domain.to_string())
}

fn write_hosts(path: &str, lines: &[String]) -> Result<(), String> {
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    let tmp = format!("{}.tmp", path);
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, Path::new(path)).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn block_site(hosts_path: &str, domain: &str) -> Result<(), String> {
    let p = Path::new(hosts_path);
    let text = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| {
            match line_domain(l) {
                Some(d) => d != domain,
                None => true,
            }
        })
        .map(|l| l.to_string())
        .collect();
    lines.push(format!("0.0.0.0 {} {}", domain, MARKER));
    lines.push(format!("::1 {} {}", domain, MARKER));
    write_hosts(hosts_path, &lines)?;
    flush_dns();
    Ok(())
}

pub fn unblock_site(hosts_path: &str, domain: &str) -> Result<(), String> {
    let p = Path::new(hosts_path);
    let text = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
    let lines: Vec<String> = text
        .lines()
        .filter(|l| {
            match line_domain(l) {
                Some(d) => d != domain,
                None => true,
            }
        })
        .map(|l| l.to_string())
        .collect();
    write_hosts(hosts_path, &lines)?;
    flush_dns();
    Ok(())
}

pub fn flush_dns() {
    let _ = Command::new("ipconfig").arg("/flushdns").status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_remove_site() {
        let dir = std::env::temp_dir();
        let path = dir.join("hosts_test.txt");
        std::fs::write(&path, "127.0.0.1 localhost\r\n").unwrap();
        let p = path.to_string_lossy().to_string();

        block_site(&p, "discord.com").unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("0.0.0.0 discord.com"));
        assert!(after.contains("::1 discord.com"));

        unblock_site(&p, "discord.com").unwrap();
        let after2 = std::fs::read_to_string(&path).unwrap();
        assert!(!after2.contains("discord.com"));
        assert!(after2.contains("localhost"));

        let _ = std::fs::remove_file(&path);
    }
}
