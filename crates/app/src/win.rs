//! Windows-specific helpers of the app shell: elevation, the dev-logs
//! console window, file explorer. All `cfg(windows)`; non-Windows builds
//! get graceful fallbacks.

#[cfg(windows)]
use {
    std::os::windows::process::CommandExt,
    std::process::{Command, Stdio},
    windows::core::{w, PCWSTR},
    windows::Win32::UI::Shell::ShellExecuteW,
    windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Is the current process running elevated (administrator)?
pub fn is_admin() -> bool {
    #[cfg(windows)]
    {
        integra_zapret::win::is_admin()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Relaunch the whole app elevated via the UAC prompt ("runas").
pub fn relaunch_as_admin() -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let Some(path) = exe.to_str() else {
            return Err("path is not unicode".into());
        };
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let verb = w!("runas");
        let file = PCWSTR(wide.as_ptr());
        let result = unsafe { ShellExecuteW(None, verb, file, None, None, SW_SHOWNORMAL) };
        if (result.0 as usize) > 32 {
            Ok(())
        } else {
            Err(integra_core::i18n::tr_with(
                "core.elevation_failed_code",
                &[("code", &(result.0 as usize).to_string())],
            ))
        }
    }
    #[cfg(not(windows))]
    {
        Err(integra_core::i18n::tr("core.elevation_windows_only"))
    }
}

/// Open a dedicated console window that tails `path` live
/// (`Get-Content -Wait`). Used by the developer mode.
pub fn open_console_tail(path: &std::path::Path) -> Result<(), String> {
    let text = path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$host.UI.RawUI.WindowTitle='Integra Dev Logs'; Get-Content -LiteralPath '{text}' -Wait -Tail 400"
    );
    #[cfg(windows)]
    {
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .creation_flags(CREATE_NEW_CONSOLE)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = script;
        Err(integra_core::i18n::tr("zapret.windows_only"))
    }
}

/// Reveal a folder in the system file manager.
pub fn open_in_explorer(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("explorer")
            .arg(path)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Show an arbitrary file's properties/folder (used by "reveal config").
pub fn reveal_file(path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        open_in_explorer(parent)?;
    }
    Ok(())
}
