use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};

pub const CREATE_NO_WINDOW_FLAG: u32 = 0x0800_0000;
pub const BELOW_NORMAL_PRIORITY_FLAG: u32 = 0x0000_4000;
pub const IDLE_PRIORITY_FLAG: u32 = 0x0000_0040;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessPriority {
    BelowNormal,
    Idle,
}

impl ProcessPriority {
    pub fn creation_flag(self) -> u32 {
        match self {
            ProcessPriority::BelowNormal => BELOW_NORMAL_PRIORITY_FLAG,
            ProcessPriority::Idle => IDLE_PRIORITY_FLAG,
        }
    }
}

/// RAII wrapper around a Windows Job Object configured with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Dropping or calling `terminate()`
/// kills the entire process tree (e.g. `yt-dlp.exe` + `ffmpeg.exe` + `deno.exe`).
pub struct JobGroup {
    #[cfg(windows)]
    handle: isize,
}

unsafe impl Send for JobGroup {}
unsafe impl Sync for JobGroup {}

impl JobGroup {
    pub fn new() -> io::Result<Self> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::JobObjects::{
                CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            };

            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }

            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            let ok = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let err = io::Error::last_os_error();
                CloseHandle(handle);
                return Err(err);
            }

            Ok(Self {
                handle: handle as isize,
            })
        }

        #[cfg(not(windows))]
        {
            Ok(Self {})
        }
    }

    pub fn assign_child(&self, child: &Child) -> io::Result<()> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
            if let Some(raw) = child.raw_handle() {
                let ok = AssignProcessToJobObject(self.handle as *mut _, raw as *mut _);
                if ok == 0 {
                    // On newer Windows, nested jobs work natively; only error if process isn't already exited
                    let err = io::Error::last_os_error();
                    return Err(err);
                }
            }
            Ok(())
        }

        #[cfg(not(windows))]
        {
            let _ = child;
            Ok(())
        }
    }

    /// Terminates every process inside this Job Object immediately.
    pub fn terminate(&self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;
            if self.handle != 0 {
                let _ = TerminateJobObject(self.handle as *mut _, 1);
            }
        }
    }
}

impl Drop for JobGroup {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            if self.handle != 0 {
                CloseHandle(self.handle as *mut _);
                self.handle = 0;
            }
        }
    }
}

/// Builds a strictly shell-free `tokio::process::Command` with `CREATE_NO_WINDOW`,
/// process priority, and full UTF-8 environment variables.
pub fn build_hidden_command<I, S>(
    exe: &Path,
    args: I,
    priority: ProcessPriority,
    cwd: Option<&Path>,
) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("NO_COLOR", "1");

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW_FLAG | priority.creation_flag());
    }

    cmd
}

/// Spawns a child process and immediately binds it to the provided `JobGroup`.
pub fn spawn_in_job<I, S>(
    job: &JobGroup,
    exe: &Path,
    args: I,
    priority: ProcessPriority,
    cwd: Option<&Path>,
) -> io::Result<Child>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = build_hidden_command(exe, args, priority, cwd);
    let child = cmd.spawn()?;
    if let Err(err) = job.assign_child(&child) {
        tracing_or_log(&format!("Aviso ao associar processo ao Job Object: {err}"));
    }
    Ok(child)
}

/// Dynamically changes a running child process priority (used by Game Mode for Workshop encodes).
pub fn set_child_priority(child: &Child, priority: ProcessPriority) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::SetPriorityClass;
        if let Some(raw) = child.raw_handle() {
            let _ = SetPriorityClass(raw as *mut _, priority.creation_flag());
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (child, priority);
    }
}

/// Runs a short hidden command to completion inside a temporary `JobGroup` and returns `(exit_code, stdout, stderr)`.
pub async fn run_hidden_output(
    exe: &Path,
    args: &[OsString],
    priority: ProcessPriority,
) -> io::Result<(i32, String, String)> {
    let job = JobGroup::new()?;
    let child = spawn_in_job(&job, exe, args, priority, None)?;
    let out = child.wait_with_output().await?;
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    Ok((code, stdout, stderr))
}

/// Enumerates currently running `.exe` process names on Windows (for Game Mode detection and UI picker).
pub fn list_running_exe_names() -> Vec<String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };

        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return Vec::new();
        }

        let mut set = BTreeSet::new();
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if name.to_ascii_lowercase().ends_with(".exe") {
                    set.insert(name);
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        set.into_iter().collect()
    }

    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Verifies WebView2 Evergreen Runtime presence on Windows before starting Tauri.
pub fn ensure_webview2_installed() -> bool {
    #[cfg(windows)]
    {
        let candidates = [
            r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
            r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        ];
        // Check via reg.exe or known Edge/WebView2 directories without extra crate bloat
        for base in [
            std::env::var_os("ProgramFiles(x86)"),
            std::env::var_os("ProgramFiles"),
            std::env::var_os("LOCALAPPDATA"),
        ]
        .into_iter()
        .flatten()
        {
            let p = Path::new(&base)
                .join("Microsoft")
                .join("EdgeWebView")
                .join("Application");
            if p.exists() {
                return true;
            }
        }
        // Fallback: check registry via hidden reg query
        let mut cmd = std::process::Command::new("reg.exe");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW_FLAG);
        }
        for key in candidates {
            if let Ok(out) = cmd
                .args(["query", &format!("HKLM\\{key}"), "/v", "pv"])
                .output()
            {
                if out.status.success() {
                    return true;
                }
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Displays a friendly native Windows dialog if WebView2 is missing instead of crashing.
pub fn show_missing_webview2_dialog() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONWARNING, MB_OK,
        };
        let title: Vec<u16> = "Boladão — WebView2 necessário\0".encode_utf16().collect();
        let body: Vec<u16> = concat!(
            "O Microsoft Edge WebView2 não foi encontrado neste Windows.\n\n",
            "Baixe e instale o Runtime Evergreen oficial da Microsoft em:\n",
            "https://go.microsoft.com/fwlink/p/?LinkId=2124703\n\n",
            "Depois de instalar, abra o Boladão novamente.\0"
        )
        .encode_utf16()
        .collect();
        MessageBoxW(std::ptr::null_mut(), body.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING);
    }
}

fn tracing_or_log(msg: &str) {
    eprintln!("[boladao] {msg}");
}
