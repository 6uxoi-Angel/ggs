//! Crash-safe file replacement helpers shared by the config and list writers.

use std::fs;
use std::io;
use std::path::Path;

/// Atomically replace `path` with `data`.
///
/// The payload goes to a UNIQUE temp file in the same parent directory
/// (`.{file_name}.tmp.{pid}.{unix_nanos}`), is fully written and flushed
/// (`sync_all`), then renamed over the target. A fixed temp name would let
/// two concurrent writers clobber each other, and skipping the fsync could
/// leave a truncated file after a crash — both are avoided here.
///
/// On rename failure (a Windows quirk while the target is briefly held open)
/// the target is removed best-effort and the rename retried once. On any
/// error the temp file never outlives the call. The parent directory is
/// created when missing (matching the previous writer behavior).
pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(".{file_name}.tmp.{}.{nanos}", std::process::id()));

    let result = (|| -> io::Result<()> {
        {
            // Inner scope drops the handle right after the flush.
            let mut file = fs::File::create(&tmp)?;
            io::Write::write_all(&mut file, data)?;
            file.sync_all()?;
        }
        match fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(_) => {
                // Windows can refuse rename-over-open-file: drop the target
                // and retry exactly once before giving up.
                let _ = fs::remove_file(path);
                fs::rename(&tmp, path)
            }
        }
    })();

    if result.is_err() {
        // Never leave a half-written temp file behind.
        let _ = fs::remove_file(&tmp);
    }

    // Best-effort directory flush so the rename itself is durable on Unix.
    #[cfg(unix)]
    {
        if result.is_ok() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::tempfile_lite::TempDir;

    #[test]
    fn atomic_write_creates_overwrites_and_leaves_no_temp() {
        let dir = TempDir::new("integra-fsutil");
        let target = dir.path().join("nested").join("config.json");

        atomic_write(&target, b"first").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"first");

        atomic_write(&target, b"second").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"second");

        let leftovers: Vec<_> = fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temp files must be cleaned up");
    }
}
