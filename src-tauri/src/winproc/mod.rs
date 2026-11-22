use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};

pub const CREATE_NO_WINDOW_FLAG: u32 = 0x0800_0000;
pub const BELOW_NORMAL_PRIORITY_FLAG: u32 = 0x0000_4000;
pub const IDLE_PRIORITY_FLAG: u32 = 0x0000_0040;
/// Starts the child with its main thread suspended, so it cannot run (or spawn grandchildren)
/// before it is inside the Job Object.
const CREATE_SUSPENDED_FLAG: u32 = 0x0000_0004;

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
            let raw = child.raw_handle().ok_or_else(|| {
                io::Error::new(io::ErrorKind::Other, "o processo já foi encerrado e não tem mais handle")
            })?;
            if AssignProcessToJobObject(self.handle as *mut _, raw as *mut _) == 0 {
                return Err(io::Error::last_os_error());
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

/// Spawns a child process that only starts running once it is inside the provided `JobGroup`.
///
/// Fail-closed: the child is created suspended, assigned to the Job Object and then resumed.
/// If either step fails, the child is terminated (unless it has already exited) and the error
/// is returned, so no process ever runs outside the job and survives the app.
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
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW_FLAG | priority.creation_flag() | CREATE_SUSPENDED_FLAG);
    }
    let mut child = cmd.spawn()?;

    let bound = job.assign_child(&child).and_then(|_| resume_main_thread(&child));
    if let Err(err) = bound {
        // `try_wait` tells apart a child that already exited from one that is still alive (suspended).
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.start_kill();
        }
        tracing_or_log(&format!("Processo encerrado: não entrou no Job Object ({err})"));
        return Err(io::Error::new(
            err.kind(),
            format!("não foi possível isolar o processo no Job Object do Windows: {err}"),
        ));
    }
    Ok(child)
}

/// Resumes every thread of a child created with `CREATE_SUSPENDED` (in practice, its main thread).
fn resume_main_thread(child: &Child) -> io::Result<()> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        };
        use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

        let pid = child
            .id()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "o processo já foi encerrado"))?;
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut resumed = 0usize;
        let mut more = Thread32First(snap, &mut entry) != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    if ResumeThread(thread) != u32::MAX {
                        resumed += 1;
                    }
                    CloseHandle(thread);
                }
            }
            more = Thread32Next(snap, &mut entry) != 0;
        }
        CloseHandle(snap);

        if resumed == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "não foi possível retomar a thread principal do processo",
            ));
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = child;
        Ok(())
    }
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    fn system32(exe: &str) -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32")
            .join(exe)
    }

    #[tokio::test]
    async fn child_starts_inside_the_job_object_and_dies_with_it() {
        use windows_sys::Win32::System::JobObjects::IsProcessInJob;

        let job = JobGroup::new().unwrap();
        let mut child = spawn_in_job(
            &job,
            &system32("PING.EXE"),
            ["-n", "30", "127.0.0.1"],
            ProcessPriority::BelowNormal,
            None,
        )
        .unwrap();

        let mut in_job = 0;
        unsafe {
            IsProcessInJob(child.raw_handle().unwrap() as _, job.handle as _, &mut in_job);
        }
        assert_ne!(in_job, 0, "o processo precisa estar no Job Object desde o início");

        // Closing the last handle of the job (app exit or crash) must kill the whole tree.
        drop(job);
        let exited = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        assert!(exited.is_ok(), "o processo sobreviveu ao fechamento do Job Object");
    }

    #[tokio::test]
    async fn suspended_child_is_resumed_and_runs_to_completion() {
        let job = JobGroup::new().unwrap();
        let out = spawn_in_job(
            &job,
            &system32("HOSTNAME.EXE"),
            Vec::<&str>::new(),
            ProcessPriority::BelowNormal,
            None,
        )
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
        assert!(out.status.success());
        assert!(!String::from_utf8_lossy(&out.stdout).trim().is_empty());
    }
}
