use crate::winproc::list_running_exe_names;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameModeOverride {
    Auto,
    ForceOn,
    ForceOff,
}

impl Default for GameModeOverride {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameModeStatus {
    pub active: bool,
    pub override_mode: GameModeOverride,
    pub reason: Option<String>,
}

pub struct GameModeMonitor {
    state: RwLock<GameModeStatus>,
    last_detected_at: RwLock<Option<Instant>>,
}

impl GameModeMonitor {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(GameModeStatus {
                active: false,
                override_mode: GameModeOverride::Auto,
                reason: None,
            }),
            last_detected_at: RwLock::new(None),
        })
    }

    pub fn status(&self) -> GameModeStatus {
        self.state.read().clone()
    }

    /// Evaluates automatic detection + 10s exit hysteresis and returns `Some(new_status)` if changed.
    pub fn poll_step(
        &self,
        auto_enabled: bool,
        override_mode: GameModeOverride,
        configured_exes: &[String],
    ) -> Option<GameModeStatus> {
        let now = Instant::now();
        let (target_active, reason) = match override_mode {
            GameModeOverride::ForceOn => (true, Some("Ativado manualmente".to_string())),
            GameModeOverride::ForceOff => (false, None),
            GameModeOverride::Auto => {
                if !auto_enabled {
                    (false, None)
                } else if let Some(why) = detect_active_game_or_fullscreen(configured_exes) {
                    *self.last_detected_at.write() = Some(now);
                    (true, Some(why))
                } else {
                    // 10-second hysteresis before exiting Game Mode (Section 11.4)
                    let last = *self.last_detected_at.read();
                    if let Some(t) = last {
                        if now.duration_since(t) < Duration::from_secs(10) {
                            let prev_reason = self.state.read().reason.clone();
                            (true, prev_reason)
                        } else {
                            *self.last_detected_at.write() = None;
                            (false, None)
                        }
                    } else {
                        (false, None)
                    }
                }
            }
        };

        let mut st = self.state.write();
        if st.active != target_active || st.override_mode != override_mode || st.reason != reason {
            st.active = target_active;
            st.override_mode = override_mode;
            st.reason = reason;
            return Some(st.clone());
        }
        None
    }
}

fn detect_active_game_or_fullscreen(configured_exes: &[String]) -> Option<String> {
    // 1. Check user-configured game executables
    if !configured_exes.is_empty() {
        let running = list_running_exe_names();
        for cfg in configured_exes {
            let target = cfg.trim().to_ascii_lowercase();
            if target.is_empty() {
                continue;
            }
            if let Some(found) = running.iter().find(|r| r.to_ascii_lowercase() == target) {
                return Some(tr!("Jogo em execução ({found})", "Game running ({found})", "Juego en ejecución ({found})"));
            }
        }
    }

    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        use windows_sys::Win32::UI::Shell::{
            SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowRect,
            GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_STYLE, WS_CAPTION,
            WS_THICKFRAME,
        };

        // 2. Direct3D exclusive fullscreen via SHQueryUserNotificationState
        let mut quns: i32 = 0;
        if SHQueryUserNotificationState(&mut quns) == 0 && quns == QUNS_RUNNING_D3D_FULL_SCREEN {
            return Some(tr!("Jogo Direct3D em tela cheia", "Full-screen Direct3D game", "Juego Direct3D en pantalla completa"));
        }

        // 3. Borderless fullscreen foreground window (excluding our own PID & Windows desktop shell)
        let hwnd = GetForegroundWindow();
        if !hwnd.is_null() && IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 {
            let mut fg_pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut fg_pid);
            if fg_pid != std::process::id() {
                let mut cls_buf = [0u16; 128];
                let cls_len = GetClassNameW(hwnd, cls_buf.as_mut_ptr(), cls_buf.len() as i32);
                let cls_name = if cls_len > 0 {
                    String::from_utf16_lossy(&cls_buf[..cls_len as usize])
                } else {
                    String::new()
                };
                let is_shell = matches!(
                    cls_name.as_str(),
                    "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
                );
                let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                let has_window_chrome = (style & (WS_CAPTION | WS_THICKFRAME)) != 0;

                if !is_shell && !has_window_chrome {
                    let mut win_rect: RECT = std::mem::zeroed();
                    if GetWindowRect(hwnd, &mut win_rect) != 0 {
                        let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                        if !hmon.is_null() {
                            let mut mi: MONITORINFO = std::mem::zeroed();
                            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                            if GetMonitorInfoW(hmon, &mut mi) != 0 {
                                let m = mi.rcMonitor;
                                if win_rect.left <= m.left
                                    && win_rect.top <= m.top
                                    && win_rect.right >= m.right
                                    && win_rect.bottom >= m.bottom
                                {
                                    return Some(tr!("Aplicativo em tela cheia", "Full-screen app", "Aplicación en pantalla completa"));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}
