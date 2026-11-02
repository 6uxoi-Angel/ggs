//! Windows-specific process & service primitives.
//!
//! Everything here is `cfg(windows)`; the manager falls back to
//! `ZapretError::WindowsOnly` on other platforms.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, TerminateProcess,
    PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};

use crate::ZapretError;

pub const CREATE_NO_WINDOW_FLAG: u32 = 0x0800_0000; // == CREATE_NO_WINDOW

#[derive(Debug, Clone)]
pub struct WinProcess {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
}

/// Snapshot all processes via Toolhelp32.
pub fn enumerate_processes() -> Vec<WinProcess> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                out.push(WinProcess {
                    pid: entry.th32ProcessID,
                    parent: entry.th32ParentProcessID,
                    name: String::from_utf16_lossy(&entry.szExeFile[..len]),
                });
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    out
}

/// PIDs of `winws.exe`, optionally only direct children of `parent`.
pub fn find_winws(parent: Option<u32>) -> Vec<u32> {
    enumerate_processes()
        .into_iter()
        .filter(|p| p.name.eq_ignore_ascii_case("winws.exe"))
        .filter(|p| parent.is_none_or(|pp| p.parent == pp))
        .map(|p| p.pid)
        .collect()
}

pub fn process_alive(pid: u32) -> bool {
    enumerate_processes().iter().any(|p| p.pid == pid)
}

pub fn terminate(pid: u32) -> Result<(), ZapretError> {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, false, pid)
            .map_err(|e| ZapretError::StopPartial(e.to_string()))?;
        let result = TerminateProcess(handle, 1);
        let _ = CloseHandle(handle);
        result.map_err(|e| ZapretError::StopPartial(e.to_string()))
    }
}

/// True when the current process has an elevated token.
pub fn is_admin() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size,
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// `std::process::Command` pre-configured to run without a console window.
pub fn hidden_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW_FLAG);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command
}

pub struct CommandOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run a hidden command, capturing output with a timeout. Optional stdin text
/// is written before waiting (used to script `service.bat` menus).
pub fn run_hidden(
    program: &str,
    args: &[&str],
    cwd: Option<&std::path::Path>,
    input: Option<&str>,
    timeout: Duration,
) -> Result<CommandOutput, ZapretError> {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(program);
    command
        .args(args)
        .creation_flags(CREATE_NO_WINDOW_FLAG)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = command.spawn().map_err(|e| ZapretError::StartFailed(e.to_string()))?;

    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        use std::io::Write;
        let _ = stdin.write_all(text.as_bytes());
        drop(stdin);
    }

    // Reader threads avoid pipe-buffer deadlocks on chatty output.
    let stdout_handle = spawn_reader(child.stdout.take());
    let stderr_handle = spawn_reader(child.stderr.take());

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ZapretError::StartFailed(format!("timeout after {timeout:?}")));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(ZapretError::StartFailed(e.to_string())),
        }
    };

    Ok(CommandOutput {
        code: status.code().unwrap_or(-1),
        stdout: stdout_handle.join().unwrap_or_default(),
        stderr: stderr_handle.join().unwrap_or_default(),
    })
}

fn spawn_reader<R: std::io::Read + Send + 'static>(
    reader: Option<R>,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let Some(mut reader) = reader else { return String::new() };
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// Query the `zapret` Windows service: (installed, running).
pub fn query_service() -> (bool, bool) {
    let output = run_hidden("sc", &["query", "zapret"], None, None, Duration::from_secs(4));
    let Ok(output) = output else { return (false, false) };
    let stdout = output.stdout.to_uppercase();
    let installed = output.code == 0 && stdout.contains("SERVICE_NAME");
    let running = installed && stdout.contains("STATE") && stdout.contains("4") && stdout.contains("RUNNING");
    (installed, running)
}

/// `sc stop/delete` a service, ignoring "not installed" errors.
pub fn sc(action: &str, service: &str) {
    let _ = run_hidden("sc", &[action, service], None, None, Duration::from_secs(20));
}

pub fn taskkill_image(image: &str) {
    let _ = run_hidden("taskkill", &["/IM", image, "/F"], None, None, Duration::from_secs(20));
}

// Keep PWSTR import meaningful for future Win32 string work.
#[allow(dead_code)]
fn _pwstr_marker(s: &mut [u16]) -> PWSTR {
    PWSTR(s.as_mut_ptr())
}

// PROCESS_QUERY_INFORMATION is used by future per-process path lookups.
#[allow(dead_code)]
const _QUERY_INFO: windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS = PROCESS_QUERY_INFORMATION;
