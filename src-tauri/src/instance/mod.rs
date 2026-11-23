//! Keeps a single Boladão running per user session. Two instances would share the same queue
//! database and download the same items twice, so a second launch only asks the open one to bring
//! its window forward and then exits. Windows uses a named event; Linux and macOS a lock file the
//! kernel releases when the process ends (even in a crash) and a local socket for the wake-up.

use std::time::Duration;

/// Passed by the updater to the new executable, followed by the PID of the process that is
/// closing, so the new version waits for it instead of finding it still open.
pub const WAIT_FOR_PID_ARG: &str = "--wait-for-pid";

#[cfg(windows)]
const EVENT_NAME: &str = r"Local\Boladao.Instance";

/// Proof that this process is the running instance. It is never released before the process
/// exits: the kernel frees the named event (or the file lock) only when the process ends.
pub struct InstanceGuard {
    #[cfg(windows)]
    event: usize,
    #[cfg(unix)]
    _lock: Option<std::fs::File>,
    #[cfg(unix)]
    listener: Option<std::os::unix::net::UnixListener>,
}

/// Returns the guard when no other instance is open in this session. When one is, asks it to
/// show its window and returns `None`.
pub fn acquire() -> Option<InstanceGuard> {
    #[cfg(windows)]
    {
        acquire_named(EVENT_NAME)
    }
    #[cfg(unix)]
    {
        acquire_in(&crate::paths::app_local_dir())
    }
}

#[cfg(unix)]
fn acquire_in(dir: &std::path::Path) -> Option<InstanceGuard> {
    use std::io::Write;
    use std::os::unix::net::{UnixListener, UnixStream};

    let socket_path = dir.join("instance.sock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("instance.lock"));
    let Ok(lock) = lock else {
        // This check never keeps the app from opening.
        eprintln!("[boladao] Não foi possível verificar se o Boladão já está aberto.");
        return Some(InstanceGuard { _lock: None, listener: None });
    };

    match lock.try_lock() {
        Ok(()) => {
            // A socket left by a crashed instance would block the bind.
            let _ = std::fs::remove_file(&socket_path);
            let listener = UnixListener::bind(&socket_path).ok();
            Some(InstanceGuard { _lock: Some(lock), listener })
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            // The open instance may still be starting: give its socket a moment to appear.
            for _ in 0..20 {
                if let Ok(mut stream) = UnixStream::connect(&socket_path) {
                    let _ = stream.write_all(b"show\n");
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            None
        }
        Err(std::fs::TryLockError::Error(e)) => {
            eprintln!("[boladao] Não foi possível verificar se o Boladão já está aberto ({e}).");
            Some(InstanceGuard { _lock: None, listener: None })
        }
    }
}

impl InstanceGuard {
    /// Runs `on_activate` every time a later launch asks this instance to come forward.
    pub fn listen<F: Fn() + Send + 'static>(self, on_activate: F) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
            use windows_sys::Win32::System::Threading::{WaitForSingleObject, INFINITE};

            if self.event == 0 {
                return;
            }
            let event = self.event;
            std::thread::spawn(move || loop {
                // SAFETY: the handle stays open for the life of the process.
                if unsafe { WaitForSingleObject(event as _, INFINITE) } != WAIT_OBJECT_0 {
                    break;
                }
                on_activate();
            });
        }
        #[cfg(unix)]
        {
            let InstanceGuard { _lock, listener } = self;
            let Some(listener) = listener else {
                return;
            };
            std::thread::spawn(move || {
                // The lock file stays open (and locked) as long as this thread lives: for good.
                let _held = _lock;
                for stream in listener.incoming() {
                    if stream.is_ok() {
                        on_activate();
                    }
                }
            });
        }
    }
}

#[cfg(windows)]
fn acquire_named(name: &str) -> Option<InstanceGuard> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS,
    };
    use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: plain Win32 calls; the handle is either closed here or kept by the guard.
    unsafe {
        // Auto-reset event: each launch that signals it wakes the listener exactly once.
        let event = CreateEventW(std::ptr::null(), 0, 0, wide.as_ptr());
        let error = GetLastError();
        if event.is_null() {
            if error == ERROR_ACCESS_DENIED {
                // Created by an instance running with other rights (for example, as administrator).
                return None;
            }
            // This check never keeps the app from opening.
            eprintln!("[boladao] Não foi possível verificar se o Boladão já está aberto (erro {error}).");
            return Some(InstanceGuard { event: 0 });
        }
        if error == ERROR_ALREADY_EXISTS {
            // This launch is what the user just clicked, so it may hand the foreground over.
            AllowSetForegroundWindow(ASFW_ANY);
            SetEvent(event);
            CloseHandle(event);
            return None;
        }
        Some(InstanceGuard { event: event as usize })
    }
}

/// When started by the updater, waits up to 20 s for the previous version to finish closing.
pub fn wait_for_previous_process() {
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == WAIT_FOR_PID_ARG {
            if let Some(pid) = args.next().and_then(|p| p.to_str()?.parse::<u32>().ok()) {
                wait_for_exit(pid, Duration::from_secs(20));
            }
            return;
        }
    }
}

fn wait_for_exit(pid: u32, timeout: Duration) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };

        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        // SAFETY: the process handle is closed before returning.
        unsafe {
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if process.is_null() {
                return; // Already gone.
            }
            WaitForSingleObject(process, millis);
            CloseHandle(process);
        }
    }
    #[cfg(unix)]
    {
        let start = std::time::Instant::now();
        // SAFETY: signal 0 only checks whether the process still exists.
        while unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 && start.elapsed() < timeout {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;

    #[test]
    fn a_second_launch_is_refused_and_wakes_the_open_instance() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire_in(dir.path()).expect("a primeira abertura precisa ficar com a instância");
        // The lock is per open file, so a second open in this same process also finds it taken.
        assert!(acquire_in(dir.path()).is_none(), "a segunda abertura precisa ser recusada");

        let (tx, rx) = std::sync::mpsc::channel();
        first.listen(move || {
            let _ = tx.send(());
        });
        assert!(acquire_in(dir.path()).is_none());
        rx.recv_timeout(Duration::from_secs(5))
            .expect("a instância aberta precisa ser avisada para mostrar a janela");
    }

    #[test]
    fn waits_for_the_closing_process_and_skips_missing_ones() {
        let mut child = std::process::Command::new("/bin/sleep").arg("1").spawn().unwrap();
        let pid = child.id();
        let reaper = std::thread::spawn(move || child.wait());
        wait_for_exit(pid, Duration::from_secs(10));
        assert!(reaper.join().unwrap().is_ok());

        let start = std::time::Instant::now();
        wait_for_exit(i32::MAX as u32 - 1, Duration::from_secs(10));
        assert!(start.elapsed() < Duration::from_secs(1), "um PID inexistente não pode travar a abertura");
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_second_launch_is_refused_and_wakes_the_open_instance() {
        let name = format!(r"Local\Boladao.Teste.{}", uuid::Uuid::new_v4());
        let first = acquire_named(&name).expect("a primeira abertura precisa ficar com a instância");
        let (tx, rx) = std::sync::mpsc::channel();
        first.listen(move || {
            let _ = tx.send(());
        });

        assert!(acquire_named(&name).is_none(), "a segunda abertura precisa ser recusada");
        rx.recv_timeout(Duration::from_secs(5))
            .expect("a instância aberta precisa ser avisada para mostrar a janela");
    }

    #[test]
    fn waits_for_the_closing_process_and_skips_missing_ones() {
        let mut child = std::process::Command::new("PING.EXE")
            .args(["127.0.0.1", "-n", "2"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("PING.EXE precisa iniciar");
        wait_for_exit(child.id(), Duration::from_secs(10));
        assert!(
            child.try_wait().expect("estado do processo").is_some(),
            "a espera só pode terminar depois que o processo fechar"
        );

        let start = Instant::now();
        wait_for_exit(u32::MAX - 1, Duration::from_secs(10));
        assert!(start.elapsed() < Duration::from_secs(1), "um PID inexistente não pode travar a abertura");
    }
}
