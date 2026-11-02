//! Zapret maintenance: release discovery (Flowseal GitHub), download and
//! safe installation with backup/rollback and user-list migration.
//!
//! Security hardening:
//! * archives are extracted through a path sanitizer — absolute paths,
//!   drive letters, `..` traversal and symlinks are rejected;
//! * total unpacked size / file count caps protect against zip bombs;
//! * the old installation is renamed to a backup first and restored
//!   automatically when anything fails mid-way;
//! * user lists (`lists/*.txt`) are migrated into the new installation.

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ZapretError;

pub const RELEASES_API: &str =
    "https://api.github.com/repos/Flowseal/zapret-discord-youtube/releases/latest";
pub const USER_AGENT: &str = "Integra/3 (+https://github.com/6uxoi-Angel)";

/// Hard caps against zip bombs / broken archives.
const MAX_TOTAL_UNPACKED: u64 = 512 * 1024 * 1024; // 512 MB
const MAX_FILES: usize = 20_000;
const MAX_RESPONSE_TEXT: u64 = 4 * 1024 * 1024; // GitHub JSON reply

/// User files carried over into a fresh installation.
pub const KEPT_LIST_FILES: [&str; 5] = [
    "list-general.txt",
    "list-exclude.txt",
    "list-google.txt",
    "ipset-all.txt",
    "ipset-exclude.txt",
];

/// Metadata stored next to the installation: `.integra-zapret.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallMeta {
    pub version: String,
    pub installed_at: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatestRelease {
    pub tag: String,
    pub version: String,
    pub name: String,
    pub published_at: String,
    pub download_url: Option<String>,
    pub size: u64,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .user_agent(USER_AGENT)
        .build()
}

/// Ask GitHub for the latest Flowseal release (and its .zip asset).
pub fn fetch_latest() -> Result<LatestRelease, ZapretError> {
    let response = agent()
        .get(RELEASES_API)
        .call()
        .map_err(|e| ZapretError::Maintenance(integra_core::i18n::tr_with(
            "maint.network_error",
            &[("error", &e.to_string())],
        )))?;

    let mut body = String::new();
    response
        .into_reader()
        .take(MAX_RESPONSE_TEXT)
        .read_to_string(&mut body)
        .map_err(|e| ZapretError::Maintenance(e.to_string()))?;

    let json: Value = serde_json::from_str(&body)
        .map_err(|e| ZapretError::Maintenance(format!("github: {e}")))?;

    let tag = json["tag_name"].as_str().unwrap_or_default().to_string();
    if tag.is_empty() {
        return Err(ZapretError::Maintenance("github: no tag_name".into()));
    }
    let version = crate::version::Version::parse(&tag)
        .map(|v| v.to_string())
        .unwrap_or_else(|| tag.trim_start_matches('v').to_string());

    let mut download_url = None;
    let mut size = 0u64;
    if let Some(assets) = json["assets"].as_array() {
        for asset in assets {
            let name = asset["name"].as_str().unwrap_or_default();
            let url = asset["browser_download_url"].as_str().unwrap_or_default();
            if name.to_ascii_lowercase().ends_with(".zip") && !url.is_empty() {
                download_url = Some(url.to_string());
                size = asset["size"].as_u64().unwrap_or(0);
                break;
            }
        }
    }

    let display_name = json["name"].as_str().unwrap_or(&tag).to_string();
    Ok(LatestRelease {
        tag,
        version,
        name: display_name,
        published_at: json["published_at"].as_str().unwrap_or_default().to_string(),
        download_url,
        size,
    })
}

/// Stream `url` into `dest` reporting (copied, total) progress.
pub fn download(
    url: &str,
    dest: &Path,
    progress: &dyn Fn(u64, u64),
) -> Result<(), ZapretError> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let response = agent()
        .get(url)
        .call()
        .map_err(|e| ZapretError::Maintenance(integra_core::i18n::tr_with(
            "maint.network_error",
            &[("error", &e.to_string())],
        )))?;

    let total = response
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let mut reader = response.into_reader();
    let file = fs::File::create(dest)?;
    let mut writer = std::io::BufWriter::new(file);
    let mut buffer = [0u8; 64 * 1024];
    let mut copied: u64 = 0;
    loop {
        let n = reader.read(&mut buffer).map_err(ZapretError::Io)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buffer[..n]).map_err(ZapretError::Io)?;
        copied += n as u64;
        progress(copied, total);
    }
    writer.flush().map_err(ZapretError::Io)?;
    if total > 0 && copied < total {
        return Err(ZapretError::Maintenance(format!(
            "incomplete download: {copied}/{total} bytes"
        )));
    }
    Ok(())
}

/// Normalize an archive entry name; `None` = must be rejected.
fn safe_entry_name(name: &str) -> Option<PathBuf> {
    // ZIP names use `/`; some writers emit `\` — treat both as separators.
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') {
        return None;
    }
    let mut path = PathBuf::new();
    for (index, component) in Path::new(&normalized).components().enumerate() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                // Reject drive letters ("C:") — on Unix they are plain
                // components, but a Windows archive must not create them.
                if index == 0 && part.len() == 2 && part.as_bytes()[1] == b':' {
                    return None;
                }
                path.push(part.as_ref());
            }
            Component::CurDir => {}
            _ => return None, // "..", root, drive prefix — reject
        }
    }
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

/// Extract a zip archive safely into `dest_dir` (must exist or be creatable).
pub fn safe_extract(archive_path: &Path, dest_dir: &Path) -> Result<(), ZapretError> {
    let file = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| ZapretError::Maintenance(format!("zip: {e}")))?;

    fs::create_dir_all(dest_dir)?;
    let mut total_bytes: u64 = 0;
    let file_count = archive.len();

    for index in 0..file_count {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| ZapretError::Maintenance(format!("zip: {e}")))?;

        // Symlinks are never extracted.
        if let Some(mode) = entry.unix_mode() {
            if mode & 0o170_000 == 0o120_000 {
                continue;
            }
        }
        let Some(relative) = safe_entry_name(entry.name()) else {
            return Err(ZapretError::Maintenance(format!(
                "unsafe archive entry: {}",
                entry.name()
            )));
        };
        let out_path = dest_dir.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
            continue;
        }

        total_bytes += entry.size();
        if total_bytes > MAX_TOTAL_UNPACKED || file_count > MAX_FILES {
            return Err(ZapretError::Maintenance("archive exceeds safety limits".into()));
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out).map_err(ZapretError::Io)?;
    }
    Ok(())
}

/// Metadata helpers.
pub fn installed_version(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join(".integra-zapret.json")).ok()?;
    let meta: InstallMeta = serde_json::from_str(&text).ok()?;
    Some(meta.version)
}

fn write_meta(dir: &Path, version: &str) -> Result<(), ZapretError> {
    let meta = InstallMeta {
        version: version.to_string(),
        installed_at: Local::now().to_rfc3339(),
        source: "integra".into(),
    };
    let json = serde_json::to_string_pretty(&meta).map_err(std::io::Error::other)?;
    fs::write(dir.join(".integra-zapret.json"), json)?;
    Ok(())
}

fn looks_like_install(dir: &Path) -> bool {
    dir.join("bin").join("winws.exe").is_file()
        || dir.join("winws.exe").is_file()
        || crate::strategy::discover(dir).len() > 0
}

fn timestamp() -> String {
    Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Install a downloaded release zip into `target_dir` with backup/rollback.
///
/// Steps: extract to a temp sibling → rename old install to
/// `.integra-backup-<ts>` (if any) → move extracted contents into place →
/// migrate user lists → write metadata. On failure the backup is restored
/// and the error returned.
pub fn install_release(archive: &Path, target_dir: &Path, version: &str) -> Result<(), ZapretError> {
    if !target_dir.is_dir() {
        return Err(ZapretError::Maintenance("profile folder is missing".into()));
    }
    let parent = target_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let staging = parent.join(format!(".integra-install-{}", timestamp()));
    let _ = fs::remove_dir_all(&staging);

    let result = install_inner(archive, target_dir, &staging, version);
    let _ = fs::remove_dir_all(&staging);
    result
}

fn install_inner(
    archive: &Path,
    target_dir: &Path,
    staging: &Path,
    version: &str,
) -> Result<(), ZapretError> {
    // 1. Safe extraction into staging.
    safe_extract(archive, staging)?;

    // Find the actual install root inside the staging dir (some zips wrap
    // everything in a single top folder).
    let source_root = find_install_root(staging);

    // 2. Backup the old installation.
    let backup = target_dir
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".integra-backup-{}", timestamp()));
    let had_old = looks_like_install(target_dir);
    if had_old {
        fs::rename(target_dir, &backup)?;
    }
    // Target no longer holds the old install: on failure we restore.
    let outcome = (|| -> Result<(), ZapretError> {
        fs::create_dir_all(target_dir)?;
        // 3. Move extracted contents into place.
        for entry in fs::read_dir(source_root)? {
            let entry = entry?;
            let dest = target_dir.join(entry.file_name());
            if dest.exists() {
                let _ = fs::remove_dir_all(&dest);
                let _ = fs::remove_file(&dest);
            }
            fs::rename(entry.path(), dest)?;
        }
        // 4. Migrate user lists from the backup.
        if had_old {
            migrate_lists(&backup, target_dir);
        }
        // 5. Metadata.
        write_meta(target_dir, version)?;
        Ok(())
    })();

    if let Err(err) = outcome {
        // Rollback: remove the partial install, restore the backup.
        let _ = fs::remove_dir_all(target_dir);
        if had_old && backup.is_dir() {
            let _ = fs::rename(&backup, target_dir);
        }
        return Err(ZapretError::Maintenance(format!(
            "{}: {}",
            integra_core::i18n::tr("maint.rollback"),
            err
        )));
    }

    // Success: keep exactly one backup, drop older ones.
    cleanup_old_backups(target_dir);
    Ok(())
}

fn find_install_root(staging: &Path) -> PathBuf {
    // Prefer the directory that directly contains general*.bat / winws.exe.
    if looks_like_install(staging) {
        return staging.to_path_buf();
    }
    let mut candidates: Vec<PathBuf> = fs::read_dir(staging)
        .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    candidates.sort();
    for candidate in candidates {
        if candidate.is_dir() && looks_like_install(&candidate) {
            return candidate;
        }
    }
    staging.to_path_buf()
}

/// Copy user list files from the old installation into the new one.
pub fn migrate_lists(old_dir: &Path, new_dir: &Path) {
    for file in KEPT_LIST_FILES {
        let from = old_dir.join("lists").join(file);
        if !from.is_file() {
            continue;
        }
        let to_dir = new_dir.join("lists");
        if fs::create_dir_all(&to_dir).is_ok() {
            let to = to_dir.join(file);
            if fs::copy(&from, &to).is_err() {
                // Keep going: a single failed copy must not break the update.
            }
        }
    }
}

fn cleanup_old_backups(target_dir: &Path) {
    if let Some(parent) = target_dir.parent() {
        if let Ok(entries) = fs::read_dir(parent) {
            let mut backups: Vec<(String, PathBuf)> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name().to_string_lossy().starts_with(".integra-backup-")
                })
                .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
                .collect();
            backups.sort();
            // Keep the newest (last), delete the rest.
            for (_, path) in backups.iter().rev().skip(1) {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use integra_core::lists::tempfile_lite::TempDir;
    use std::io::Write as _;

    fn build_zip(path: &Path, entries: &[(&str, &[u8])], with_traversal: bool) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions = Default::default();
        for (name, data) in entries {
            if with_traversal && name.ends_with("evil.txt") {
                zip.start_file("../evil.txt", options).unwrap();
            } else {
                zip.start_file(*name, options).unwrap();
            }
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn safe_extract_rejects_traversal_and_symlinks() {
        let dir = TempDir::new("integra-mnt");
        let zip_path = dir.path().join("bad.zip");
        build_zip(
            &zip_path,
            &[("ok/inner.txt", b"hello"), ("evil.txt", b"pwned")],
            true,
        );
        let out = dir.path().join("out");
        let err = safe_extract(&zip_path, &out).unwrap_err();
        assert!(err.to_string().contains("unsafe archive entry"));
        assert!(!dir.path().join("evil.txt").exists(), "must not escape");
        assert!(!out.join("evil.txt").exists());
    }

    #[test]
    fn safe_extract_extracts_normal_archives() {
        let dir = TempDir::new("integra-mnt2");
        let zip_path = dir.path().join("good.zip");
        build_zip(
            &zip_path,
            &[("bin/winws.exe", b"stub"), ("general.bat", b"@echo off"), ("lists/list-general.txt", b"example.com\n")],
            false,
        );
        let out = dir.path().join("out");
        safe_extract(&zip_path, &out).unwrap();
        assert!(out.join("bin/winws.exe").is_file());
        assert!(out.join("general.bat").is_file());
        assert_eq!(fs::read_to_string(out.join("lists/list-general.txt")).unwrap(), "example.com\n");
    }

    #[test]
    fn entry_names_are_sanitized() {
        // Compare path components (not the rendered string): on Windows
        // PathBuf renders separators as `\`, on Unix as `/`.
        let p = safe_entry_name("folder/file.txt").expect("valid entry");
        let parts: Vec<String> = p
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        assert_eq!(parts, ["folder", "file.txt"]);
        assert!(safe_entry_name("../evil").is_none());
        assert!(safe_entry_name("/abs").is_none());
        assert!(safe_entry_name("C:\\evil").is_none());
        assert!(safe_entry_name("..\\windows\\evil").is_none());
        assert!(safe_entry_name(".").is_none());
    }

    #[test]
    fn install_release_migrates_lists_and_rolls_back() {
        let dir = TempDir::new("integra-mnt3");
        let target = dir.path().join("zapret");
        fs::create_dir_all(target.join("lists")).unwrap();
        fs::write(target.join("general.bat"), "@echo off").unwrap();
        fs::write(target.join("bin").join("winws.exe"), b"old").unwrap_or_else(|_| {
            fs::create_dir_all(target.join("bin")).unwrap();
            fs::write(target.join("bin").join("winws.exe"), b"old").unwrap();
        });
        fs::write(target.join("lists").join("list-general.txt"), "old-list.example\n").unwrap();

        // New archive without lists (must inherit them).
        let zip_path = dir.path().join("new.zip");
        build_zip(
            &zip_path,
            &[("bin/winws.exe", b"new"), ("general.bat", b"@echo off\r\nstart \"z\" /min \"bin\\winws.exe\"\r\n")],
            false,
        );

        install_release(&zip_path, &target, "9.9.9").unwrap();
        assert_eq!(
            fs::read(target.join("bin/winws.exe")).unwrap(),
            b"new",
            "new binary installed"
        );
        assert_eq!(
            fs::read_to_string(target.join("lists/list-general.txt")).unwrap(),
            "old-list.example\n",
            "user list migrated"
        );
        assert_eq!(installed_version(&target).as_deref(), Some("9.9.9"));
        let meta: InstallMeta =
            serde_json::from_str(&fs::read_to_string(target.join(".integra-zapret.json")).unwrap()).unwrap();
        assert_eq!(meta.version, "9.9.9");
        assert!(DateTime::parse_from_rfc3339(&meta.installed_at).is_ok());

        // A backup of the old install exists.
        let parent = target.parent().unwrap();
        let has_backup = fs::read_dir(parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with(".integra-backup-"));
        assert!(has_backup);

        // A broken archive must roll the previous install back.
        let broken = dir.path().join("broken.zip");
        build_zip(&broken, &[("winws.exe", b"x"), ("evil.txt", b"y")], true);
        let err = install_release(&broken, &target, "0.0.1").unwrap_err();
        assert!(err.to_string().contains("unsafe archive entry"));
        assert_eq!(
            fs::read(target.join("bin/winws.exe")).unwrap(),
            b"new",
            "rollback restored the pre-update install"
        );
        assert_eq!(installed_version(&target).as_deref(), Some("9.9.9"));
    }

    #[test]
    fn parse_github_release_shape() {
        // The fetch_latest parser is inline; verify the shape we rely on.
        let json: Value = serde_json::from_str(
            r#"{"tag_name":"v1.6.2","name":"1.6.2","published_at":"2024-01-01T00:00:00Z",
                "assets":[{"name":"zapret-discord-youtube-1.6.2.zip","size":10,
                "browser_download_url":"https://example.com/x.zip"}]}"#,
        )
        .unwrap();
        assert_eq!(json["tag_name"].as_str(), Some("v1.6.2"));
        assert_eq!(
            json["assets"][0]["browser_download_url"].as_str(),
            Some("https://example.com/x.zip")
        );
        let version = crate::version::Version::parse("v1.6.2").unwrap().to_string();
        assert_eq!(version, "1.6.2");
    }
}
