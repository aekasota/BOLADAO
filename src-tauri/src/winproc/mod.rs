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
#[cfg(windows)]
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

    /// `nice` value on Linux and macOS: below normal is a little lower, idle the lowest.
    #[cfg(unix)]
    fn nice(self) -> libc::c_int {
        match self {
            ProcessPriority::BelowNormal => 5,
            ProcessPriority::Idle => 19,
        }
    }
}

/// The processes of one download, killed together on cancel, pause or exit.
///
/// On Windows it is a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: dropping it or
/// calling `terminate()` kills the whole tree (`yt-dlp.exe` + `ffmpeg.exe` + `deno.exe`). On
/// Linux and macOS every child starts its own process group, which takes in whatever it spawns,
/// and `terminate()` kills those groups.
pub struct JobGroup {
    #[cfg(windows)]
    handle: isize,
    #[cfg(unix)]
    groups: parking_lot::Mutex<UnixGroups>,
}

#[cfg(unix)]
#[derive(Default)]
struct UnixGroups {
    pgids: Vec<libc::pid_t>,
    terminated: bool,
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

        #[cfg(unix)]
        {
            Ok(Self {
                groups: parking_lot::Mutex::new(UnixGroups::default()),
            })
        }
    }

    pub fn assign_child(&self, child: &Child) -> io::Result<()> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
            let raw = child.raw_handle().ok_or_else(|| {
                io::Error::new(io::ErrorKind::Other, tr!("o processo já foi encerrado e não tem mais handle", "the process has already ended and has no handle", "el proceso ya terminó y no tiene handle"))
            })?;
            if AssignProcessToJobObject(self.handle as *mut _, raw as *mut _) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        #[cfg(unix)]
        {
            // The child was started with `process_group(0)`: its group id is its pid.
            let pid = child.id().ok_or_else(|| {
                io::Error::new(io::ErrorKind::Other, tr!("o processo já foi encerrado", "the process has already ended", "el proceso ya terminó"))
            })? as libc::pid_t;
            let mut groups = self.groups.lock();
            if groups.terminated {
                // A cancel or pause arrived while the child was starting.
                // SAFETY: plain syscall; a negative pid addresses the child's own group.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    tr!("o download foi interrompido", "the download was stopped", "la descarga se detuvo"),
                ));
            }
            // Groups whose processes all ended are dropped, so their ids are never reused here.
            // SAFETY: signal 0 only checks that the group still exists.
            groups.pgids.retain(|&g| unsafe { libc::kill(-g, 0) } == 0);
            groups.pgids.push(pid);
            Ok(())
        }
    }

    /// Terminates every process of this download immediately.
    pub fn terminate(&self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;
            if self.handle != 0 {
                let _ = TerminateJobObject(self.handle as *mut _, 1);
            }
        }
        #[cfg(unix)]
        {
            let mut groups = self.groups.lock();
            groups.terminated = true;
            for pgid in groups.pgids.drain(..) {
                // SAFETY: plain syscall; a negative pid addresses the whole process group.
                unsafe { libc::kill(-pgid, libc::SIGKILL) };
            }
        }
    }
}

impl Drop for JobGroup {
    fn drop(&mut self) {
        // Same as closing the Job Object on Windows: nothing of this download outlives it.
        #[cfg(unix)]
        self.terminate();
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
    #[cfg(unix)]
    {
        let nice = priority.nice();
        let background = priority == ProcessPriority::Idle;
        // SAFETY: only async-signal-safe calls run between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                libc::setpriority(libc::PRIO_PROCESS, 0, nice);
                // macOS also slows the disk and network of a background process.
                #[cfg(target_os = "macos")]
                if background {
                    libc::setpriority(libc::PRIO_DARWIN_PROCESS, 0, libc::PRIO_DARWIN_BG);
                }
                #[cfg(not(target_os = "macos"))]
                let _ = background;
                Ok(())
            });
        }
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
    // Its own process group, so killing the group also reaches FFmpeg and Deno started by yt-dlp.
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn()?;

    let bound = job.assign_child(&child).and_then(|_| resume_main_thread(&child));
    if let Err(err) = bound {
        // `try_wait` tells apart a child that already exited from one that is still alive (suspended).
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.start_kill();
        }
        tracing_or_log(&format!("Processo encerrado: não entrou no grupo do download ({err})"));
        return Err(io::Error::new(
            err.kind(),
            tr!(
                "não foi possível isolar o processo do download: {err}",
                "couldn't isolate the download process: {err}",
                "no se pudo aislar el proceso de la descarga: {err}"
            ),
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
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, tr!("o processo já foi encerrado", "the process has already ended", "el proceso ya terminó")))?;
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
                tr!(
                "não foi possível retomar a thread principal do processo",
                "couldn't resume the main thread of the process",
                "no se pudo reanudar el hilo principal del proceso"
            ),
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
    // Lowering always works; raising it back needs privileges, so it may stay low until the end.
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: plain syscall on a pid this app started.
        unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, priority.nice()) };
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

/// Names of the running programs, for Game Mode detection and its picker: the `.exe` files on
/// Windows, the executable names on Linux (games under Proton show up as their `.exe`) and macOS.
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

    #[cfg(target_os = "linux")]
    {
        let mut set = BTreeSet::new();
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            // argv[0] keeps the full name (`comm` stops at 15 characters); Wine passes Windows paths.
            let from_cmdline = std::fs::read(entry.path().join("cmdline")).ok().and_then(|raw| {
                let argv0 = raw.split(|&b| b == 0).next()?;
                let argv0 = String::from_utf8_lossy(argv0);
                let base = argv0.rsplit(['/', '\\']).next()?.trim().to_string();
                (!base.is_empty()).then_some(base)
            });
            let name = from_cmdline.or_else(|| {
                std::fs::read_to_string(entry.path().join("comm"))
                    .ok()
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty())
            });
            if let Some(name) = name {
                set.insert(name);
            }
        }
        set.into_iter().collect()
    }

    #[cfg(target_os = "macos")]
    {
        let Ok(output) = std::process::Command::new("/bin/ps").args(["-axo", "comm="]).output() else {
            return Vec::new();
        };
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| Some(Path::new(line.trim()).file_name()?.to_string_lossy().into_owned()))
            .filter(|name| !name.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

/// Verifies WebView2 Evergreen Runtime presence on Windows before starting Tauri.
pub fn ensure_webview2_installed() -> bool {
    #[cfg(windows)]
    {
        // Per-machine (64- and 32-bit views) and per-user installs, as Microsoft documents them.
        let candidates = [
            r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
            r"HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
            r"HKCU\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
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
        // Fallback: check the registry with a hidden `reg query`, one fresh command per key
        // (reusing a `Command` would pile up the arguments of every previous query).
        for key in candidates {
            let mut cmd = std::process::Command::new("reg.exe");
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(CREATE_NO_WINDOW_FLAG);
            }
            if let Ok(out) = cmd.args(["query", key, "/v", "pv"]).output() {
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
        let title = tr!(
            "Boladão — WebView2 necessário",
            "Boladão — WebView2 required",
            "Boladão — se necesita WebView2"
        );
        let body = tr!(
            "O Microsoft Edge WebView2 não foi encontrado neste Windows.\n\nBaixe e instale o Runtime Evergreen oficial da Microsoft em:\nhttps://go.microsoft.com/fwlink/p/?LinkId=2124703\n\nDepois de instalar, abra o Boladão novamente.",
            "Microsoft Edge WebView2 was not found on this Windows.\n\nDownload and install the official Evergreen Runtime from Microsoft at:\nhttps://go.microsoft.com/fwlink/p/?LinkId=2124703\n\nAfter installing it, open Boladão again.",
            "No se encontró Microsoft Edge WebView2 en este Windows.\n\nDescarga e instala el Runtime Evergreen oficial de Microsoft en:\nhttps://go.microsoft.com/fwlink/p/?LinkId=2124703\n\nDespués de instalarlo, vuelve a abrir Boladão."
        );
        let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let body: Vec<u16> = body.encode_utf16().chain(std::iter::once(0)).collect();
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

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    fn group_alive(pgid: libc::pid_t) -> bool {
        // SAFETY: signal 0 only checks that the group exists.
        unsafe { libc::kill(-pgid, 0) == 0 }
    }

    #[tokio::test]
    async fn terminate_kills_the_child_and_what_it_started() {
        let job = JobGroup::new().unwrap();
        // The shell starts a grandchild, like yt-dlp starting FFmpeg.
        let mut child = spawn_in_job(
            &job,
            &PathBuf::from("/bin/sh"),
            ["-c", "sleep 30 & wait"],
            ProcessPriority::BelowNormal,
            None,
        )
        .unwrap();
        let pgid = child.id().unwrap() as libc::pid_t;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(group_alive(pgid));

        job.terminate();
        let exited = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        assert!(exited.is_ok(), "o processo sobreviveu ao cancelamento");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!group_alive(pgid), "o neto continuou rodando depois do cancelamento");
    }

    #[tokio::test]
    async fn a_child_started_after_terminate_never_runs() {
        let job = JobGroup::new().unwrap();
        job.terminate();
        let spawned = spawn_in_job(
            &job,
            &PathBuf::from("/bin/sleep"),
            ["30"],
            ProcessPriority::Idle,
            None,
        );
        assert!(spawned.is_err());
    }

    #[tokio::test]
    async fn short_commands_run_to_completion() {
        let (code, stdout, _) =
            run_hidden_output(&PathBuf::from("/bin/echo"), &["boladão".into()], ProcessPriority::BelowNormal)
                .await
                .unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout.trim(), "boladão");
    }

    #[test]
    fn lists_running_programs() {
        assert!(!list_running_exe_names().is_empty());
    }
}
