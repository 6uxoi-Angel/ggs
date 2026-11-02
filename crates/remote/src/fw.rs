//! Firewall rules + process killing (Windows).

use std::process::Command;

/// Normalize a path for stable matching and rule names.
fn normalize(p: &str) -> String {
    p.replace('\\', "/").to_lowercase()
}

fn rule_name(canon: &str) -> String {
    // FNV-1a 32-bit -> 8 hex chars
    let mut h: u32 = 0x811c_9dc5;
    for b in canon.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("NetGuard-{:08x}", h)
}

fn netsh(args: &[&str]) -> Result<(), String> {
    let out = Command::new("netsh").args(args).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { format!("netsh exit {:?}", out.status.code()) } else { err })
    }
}

/// Block an application by its exe path. Returns the firewall rule name.
pub fn block_app(path: &str) -> Result<String, String> {
    let canon = normalize(path);
    let rule = rule_name(&canon);
    netsh(&[
        "advfirewall", "firewall", "add", "rule",
        &format!("name={}", rule),
        "dir=out",
        &format!("program=\"{}\"", path),
        "action=block",
        "profile=any",
    ])?;
    let _ = kill_by_path(path);
    Ok(rule)
}

pub fn unblock_app(path: &str) -> Result<(), String> {
    let canon = normalize(path);
    let rule = rule_name(&canon);
    netsh(&[
        "advfirewall", "firewall", "delete", "rule",
        &format!("name={}", rule),
    ])
}

/// Force-kill processes whose image name matches the exe basename.
/// Used so a blocked app looks like it simply has "no connection".
pub fn kill_by_path(path: &str) -> Result<(), String> {
    let base = std::path::Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    if base.is_empty() {
        return Ok(());
    }
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", &base])
        .status();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_name_stable_and_short() {
        let a = rule_name(&normalize(r"C:\Program Files\Discord\Discord.exe"));
        let b = rule_name(&normalize("c:/program files/discord/discord.exe"));
        assert_eq!(a, b);
        assert_eq!(a.len(), 17);
    }
}
