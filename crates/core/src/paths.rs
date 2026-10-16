//! Application paths for source runs and packaged (single-file) builds.

use std::path::{Path, PathBuf};

/// Directory that "contains the app": executable dir for packaged builds,
/// workspace root for source runs (target/debug/… → repo root is not assumed;
/// we use the crate manifest dir's parent chain up to the workspace).
pub fn application_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if is_packaged(&exe) {
            return exe.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        }
    }
    // Source run: CARGO_MANIFEST_DIR points at crates/<name>.
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(manifest);
        if let Some(root) = p.parent().and_then(Path::parent) {
            return root.to_path_buf();
        }
    }
    PathBuf::from(".")
}

fn is_packaged(exe: &Path) -> bool {
    // Heuristic: packaged binaries live outside `target/`.
    !exe.to_string_lossy().contains("target")
}

/// Resolve a bundled resource (e.g. `browser-extension`) next to the app.
pub fn resource_path(relative: &str) -> PathBuf {
    application_root().join(relative)
}

/// Per-user writable state dir: `%LOCALAPPDATA%\Integra` on Windows.
pub fn state_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join(crate::consts::APP_NAME);
        }
    }
    crate::config::config_dir().join(crate::consts::APP_NAME)
}

pub fn logs_dir() -> PathBuf {
    state_dir().join("logs")
}

pub fn crash_log_path() -> PathBuf {
    logs_dir().join("crash.log")
}
