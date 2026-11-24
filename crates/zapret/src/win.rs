//! Windows-specific process & service primitives.
//!
//! Everything here is `cfg(windows)`; the manager falls back to
//! `ZapretError::WindowsOnly` on other platforms.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use windows::core::PWSTR;
use windows::Win32::Foundation::{
    CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, TerminateProcess,
    WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

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
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
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

/// A process we hold an open OS handle to.
///
/// Termination and liveness checks go through the handle instead of the raw
/// PID, so a recycled PID can never make us kill or misreport an unrelated
/// process. All handles are owned (spawned children are duplicated via
/// `DuplicateHandle`), and `Drop` closes the handle.
pub struct OwnedProcess {
    pid: u32,
    handle: isize,
}

impl OwnedProcess {
    /// Take ownership of a spawned child's process handle. The handle is
    /// duplicated, so the `Child` may outlive or predecease this value.
    pub fn from_child(child: &Child) -> Option<OwnedProcess> {
        use std::os::windows::io::AsRawHandle;
        let mut duplicated = HANDLE::default();
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                HANDLE(child.as_raw_handle()),
                GetCurrentProcess(),
                &mut duplicated,
                0,
                false.into(),
                DUPLICATE_SAME_ACCESS,
            )
            .ok()?;
        }
        Some(Self {
            pid: child.id(),
            handle: duplicated.0 as isize,
        })
    }

    /// Open an existing process by PID (discovered winws children, orphans).
    /// `None` when the process is gone or the handle cannot be opened.
    pub fn from_pid(pid: u32) -> Option<OwnedProcess> {
        unsafe {
            let handle = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
            .ok()?;
            Some(Self {
                pid,
                handle: handle.0 as isize,
            })
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.handle as *mut std::ffi::c_void)
    }

    /// True while the process is still running (`WaitForSingleObject`, 0 ms).
    pub fn is_running(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle(), 0) == WAIT_TIMEOUT }
    }

    /// Terminate with exit code 1 and wait up to 5 s for the exit.
    /// Returns false when the process could not be confirmed dead.
    pub fn terminate(&self) -> bool {
        unsafe {
            // "already exited" races are non-fatal: the bounded wait below
            // decides the outcome.
            let _ = TerminateProcess(self.handle(), 1);
            WaitForSingleObject(self.handle(), 5000) == WAIT_OBJECT_0
        }
    }

    /// Case-insensitive check that the opened process image contains
    /// `needle` (e.g. `winws.exe`) — a last guard against PID reuse for
    /// handles that came from [`OwnedProcess::from_pid`].
    pub fn matches_image(&self, needle: &str) -> bool {
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        unsafe {
            if QueryFullProcessImageNameW(
                self.handle(),
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut size,
            )
            .is_err()
            {
                return false;
            }
        }
        let len = (size as usize).min(buf.len());
        let image = String::from_utf16_lossy(&buf[..len]);
        image.to_lowercase().contains(&needle.to_lowercase())
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle());
        }
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
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = command
        .spawn()
        .map_err(|e| ZapretError::StartFailed(e.to_string()))?;

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
                    return Err(ZapretError::StartFailed(format!(
                        "timeout after {timeout:?}"
                    )));
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
        let Some(mut reader) = reader else {
            return String::new();
        };
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// Lifecycle state of a Windows service as reported by `sc query`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    /// Output could not be parsed (or the service is absent) — never guess.
    Unknown,
    Stopped,
    StartPending,
    StopPending,
    Running,
    PausePending,
    Paused,
}

/// Parse the `STATE : <code> <WORD>` line of `sc query` output.
///
/// The state WORD after the numeric code decides the result; the code alone
/// is never trusted (a naive `contains("4")` also matched PIDs, dates and
/// byte offsets elsewhere in the output). The label word itself is localized
/// on non-English Windows (`СОСТОЯНИЕ` on RU builds), so instead of keying on
/// the literal `STATE` we scan every `label : <code> <WORD>` line and accept
/// only a known state token. Unparsable output maps to
/// [`ServiceState::Unknown`], never to a guessed running/stopped value.
fn parse_sc_query_state(stdout: &str) -> ServiceState {
    for line in stdout.lines() {
        let Some(colon) = line.find(':') else {
            continue;
        };
        let mut tokens = line[colon + 1..].split_whitespace();
        let _code = tokens.next(); // "4", "2", ... — deliberately ignored
        let Some(word) = tokens.next() else {
            continue;
        };
        match word.to_ascii_uppercase().as_str() {
            "RUNNING" => return ServiceState::Running,
            "STOPPED" => return ServiceState::Stopped,
            "START_PENDING" => return ServiceState::StartPending,
            "STOP_PENDING" => return ServiceState::StopPending,
            "PAUSE_PENDING" => return ServiceState::PausePending,
            "PAUSED" => return ServiceState::Paused,
            _ => continue,
        }
    }
    ServiceState::Unknown
}

/// Query a Windows service by name: (installed, [`ServiceState`]).
pub fn query_service_ex(service: &str) -> (bool, ServiceState) {
    let Ok(output) = run_hidden(
        "sc",
        &["query", service],
        None,
        None,
        Duration::from_secs(4),
    ) else {
        return (false, ServiceState::Unknown);
    };
    let state = parse_sc_query_state(&output.stdout);
    let installed = output.code == 0 && state != ServiceState::Unknown;
    (installed, state)
}

/// Query the `zapret` Windows service: (installed, running).
pub fn query_service() -> (bool, bool) {
    let (installed, state) = query_service_ex("zapret");
    (installed, state == ServiceState::Running)
}

/// `sc stop/delete` a service, ignoring "not installed" errors.
pub fn sc(action: &str, service: &str) {
    let _ = run_hidden(
        "sc",
        &[action, service],
        None,
        None,
        Duration::from_secs(20),
    );
}

// Keep PWSTR import meaningful for future Win32 string work.
#[allow(dead_code)]
fn _pwstr_marker(s: &mut [u16]) -> PWSTR {
    PWSTR(s.as_mut_ptr())
}

// PROCESS_QUERY_INFORMATION is used by future per-process path lookups.
#[allow(dead_code)]
const _QUERY_INFO: windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS =
    PROCESS_QUERY_INFORMATION;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sc_query_state_parsed_from_word_not_code() {
        assert_eq!(
            parse_sc_query_state("SERVICE_NAME zapret\r\n        TYPE               : 10  WIN32_OWN_PROCESS\r\n        STATE              : 4  RUNNING\r\n        PID                4242"),
            ServiceState::Running
        );
        assert_eq!(
            parse_sc_query_state("\n        STATE              : 1  STOPPED\n"),
            ServiceState::Stopped
        );
        // Pending / paused states are explicit non-running variants.
        assert_eq!(
            parse_sc_query_state("STATE              : 2  START_PENDING"),
            ServiceState::StartPending
        );
        assert_eq!(
            parse_sc_query_state("STATE              : 7  PAUSED"),
            ServiceState::Paused
        );
        // The state label is localized on non-English Windows; the state
        // WORD stays English — parsing must still work.
        assert_eq!(
            parse_sc_query_state(
                "ИМЯ_СЛУЖБЫ: zapret\n        ТИП               : 10  WIN32_OWN_PROCESS  \n        СОСТОЯНИЕ         : 4  RUNNING\n"
            ),
            ServiceState::Running
        );
        // Unparsable output must never look like a known state.
        assert_eq!(parse_sc_query_state("no state here"), ServiceState::Unknown);
        assert_eq!(
            parse_sc_query_state("STATE    garbage"),
            ServiceState::Unknown
        );
        assert_eq!(
            parse_sc_query_state("STATE : 99 WHAT"),
            ServiceState::Unknown
        );
        // A stray "4" elsewhere in the output must not decide the result.
        assert_eq!(
            parse_sc_query_state("PID                4\n"),
            ServiceState::Unknown
        );
    }
}
