// First, so the `tr!` macro is in scope in every module below.
#[macro_use]
pub mod i18n;

pub mod args;
pub mod bins;
pub mod classify;
pub mod cookies;
pub mod errors;
pub mod gamemode;
#[cfg(windows)]
pub mod installer;
pub mod instance;
pub mod ipc;
pub mod job;
pub mod library;
pub mod limiter;
pub mod paths;
pub mod probe;
pub mod queue;
pub mod settings;
pub mod spec;
pub mod updater;
pub mod winproc;
pub mod workshop;

use crate::bins::BinManager;
use crate::gamemode::GameModeMonitor;
#[cfg(windows)]
use crate::installer::{detect_launch_mode, ensure_webview2_or_bootstrap, LaunchMode};
use crate::ipc::AppState;
use crate::library::{spectrograms_cache_dir, waveforms_cache_dir, LibraryStore};
use crate::limiter::{BandwidthLimiter, BandwidthProfile};
use crate::paths::{reveal_in_explorer, APP_NAME};
use crate::probe::{thumbs_cache_dir, ProbeService};
use crate::queue::QueueManager;
use crate::settings::{load_or_init_settings, save_settings_to_disk};
use chrono::Utc;
use parking_lot::RwLock;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tauri::http::{header, Response, StatusCode};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_notification::NotificationExt;

#[cfg(windows)]
fn run_setup_init() {
    let cfg = load_or_init_settings();
    let _ = fs::create_dir_all(&cfg.music_root);
    let _ = fs::create_dir_all(&cfg.video_root);
    let _ = LibraryStore::open_default();

    if let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        let bins = BinManager::new();
        let _ = rt.block_on(bins.ensure_ready(None, cfg.ytdlp_channel));
    }
}

#[cfg(windows)]
fn run_installer_or_uninstaller_ui(launch_mode: LaunchMode) {
    if !ensure_webview2_or_bootstrap() {
        winproc::show_missing_webview2_dialog();
        return;
    }

    let rt = tokio::runtime::Runtime::new().expect("Falha iniciando runtime Tokio");
    tauri::async_runtime::set(rt.handle().clone());

    let mode_for_setup = launch_mode.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(launch_mode)
        .setup(move |app| {
            let init_json =
                serde_json::to_string(&mode_for_setup).unwrap_or_else(|_| "{\"mode\":\"installer\"}".to_string());
            let init_script = format!("window.__BOLADAO_LAUNCH__ = {init_json};");
            let (title, w, h) = match &mode_for_setup {
                LaunchMode::Installer => (tr!("Instalar Boladão", "Install Boladão", "Instalar Boladão"), 760.0, 548.0),
                LaunchMode::Uninstaller { .. } => (
                    tr!("Desinstalar Boladão", "Uninstall Boladão", "Desinstalar Boladão"),
                    640.0,
                    440.0,
                ),
                LaunchMode::App => (APP_NAME.to_string(), 1120.0, 740.0),
            };

            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(title)
                .inner_size(w, h)
                .decorations(false)
                .shadow(true)
                .resizable(false)
                .center()
                .background_color(Color(14, 14, 16, 255))
                .initialization_script(&init_script)
                .build()?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::get_launch_mode,
            ipc::get_system_language,
            ipc::get_installer_defaults,
            ipc::install_boladao,
            ipc::launch_installed_boladao,
            ipc::uninstall_boladao,
            ipc::close_uninstaller,
        ])
        .run(tauri::generate_context!())
        .expect("Erro executando instalador Boladão");
}

pub fn run() {
    // WebKitGTK's DMA-BUF renderer leaves parts of the page unpainted (popovers, sometimes the
    // whole window) with NVIDIA drivers, in virtual machines, under WSL and in the AppImage, which
    // carries its own libraries. The older path paints everywhere. The variable only counts when
    // it is in the environment the process starts with (setting it here is too late inside the
    // AppImage), so the app restarts itself once with it; whoever sets it keeps their choice.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        use std::os::unix::process::CommandExt;
        if let Ok(exe) = std::env::current_exe() {
            // Only returns when the restart fails; the app then goes on without it.
            let err = std::process::Command::new(exe)
                .args(std::env::args_os().skip(1))
                .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
                .exec();
            eprintln!("[boladao] Não foi possível reabrir com o renderizador compatível: {err}");
        }
    }

    // Messages shown before the settings load (installer, missing WebView2) follow the system.
    i18n::set(i18n::system());

    // Windows installs itself (the app is its own installer); Linux and macOS use their packages.
    #[cfg(windows)]
    {
        if std::env::args().any(|arg| arg == "--setup-init") {
            run_setup_init();
            return;
        }

        let launch_mode = detect_launch_mode();
        if !matches!(launch_mode, LaunchMode::App) {
            run_installer_or_uninstaller_ui(launch_mode);
            return;
        }
    }

    // One Boladão per session: a second launch only brings the open window forward, before it
    // touches the queue database or applies an update.
    instance::wait_for_previous_process();
    let Some(single_instance) = instance::acquire() else {
        eprintln!("[boladao] O Boladão já está aberto; a janela dele foi trazida para a frente.");
        return;
    };

    // An update downloaded and verified in the previous session is installed before anything opens.
    updater::cleanup_previous_binary();
    match updater::apply_pending(true) {
        Ok(true) => return,
        Ok(false) => {}
        Err(e) => eprintln!("[boladao] Atualização não aplicada: {e}"),
    }
    #[cfg(windows)]
    {
        installer::sync_registered_version();

        if !ensure_webview2_or_bootstrap() {
            winproc::show_missing_webview2_dialog();
            return;
        }
    }

    let settings = Arc::new(RwLock::new(load_or_init_settings()));
    i18n::apply(settings.read().language);
    {
        let cfg = settings.read();
        let _ = fs::create_dir_all(&cfg.music_root);
        let _ = fs::create_dir_all(&cfg.video_root);
        // `.boladao-tmp` is cleaned by `QueueManager::open_default`, after the queue is loaded,
        // so partial downloads of interrupted jobs survive a restart.
    }
    cookies::secure_cookie_store();

    let bins = BinManager::new();
    let probe = Arc::new(ProbeService::new());
    let gamemode = GameModeMonitor::new();
    let library = Arc::new(
        LibraryStore::open_default().expect("Não foi possível inicializar banco SQLite da Biblioteca"),
    );

    let rt = tokio::runtime::Runtime::new().expect("Falha iniciando runtime Tokio");
    tauri::async_runtime::set(rt.handle().clone());

    // The 7-day peak is the highest of the daily peaks still inside the window.
    let today_peak = {
        let mut cfg = settings.write();
        let today = chrono::Local::now().date_naive();
        cfg.peak_7d_bps = limiter::prune_daily_peaks(&mut cfg.daily_peaks, today);
        cfg.daily_peaks.get(&today.to_string()).copied().unwrap_or(0)
    };

    let limiter = rt.block_on(async {
        let cfg = settings.read();
        BandwidthLimiter::start(
            cfg.bandwidth_profile,
            cfg.custom_limit_bps,
            cfg.light_limit_bps,
            cfg.peak_7d_bps,
        )
        .await
    });
    limiter.seed_today_peak(today_peak);

    let queue = rt.block_on(async {
        QueueManager::open_default(
            bins.clone(),
            limiter.clone(),
            gamemode.clone(),
            library.clone(),
            settings.clone(),
        )
        .expect("Não foi possível inicializar banco SQLite da Fila")
    });

    let app_state = AppState {
        bins: bins.clone(),
        probe,
        limiter: limiter.clone(),
        gamemode: gamemode.clone(),
        library,
        queue: queue.clone(),
        settings: settings.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Custom CSP-strict local media protocol (`http://boladaomedia.localhost/{thumb|wave|spec}/<file>`)
        .register_uri_scheme_protocol("boladaomedia", |ctx, request| {
            // Tauri serves custom protocols to every webview, including the login windows, which
            // load the platforms' own pages and their third-party scripts. Only the app reads media.
            if ctx.webview_label() != "main" {
                return Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(Vec::new())
                    .unwrap();
            }

            let path = request.uri().path().trim_start_matches('/');
            let mut parts = path.splitn(2, '/');
            let kind = parts.next().unwrap_or("");
            let fname = parts.next().unwrap_or("");

            // Only names the app generates itself, which also rules out traversal (`..`, `/`, `\`)
            // and drive-relative names such as `C:arquivo`.
            if !is_media_cache_name(fname) {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Vec::new())
                    .unwrap();
            }

            let file_path = match kind {
                "thumb" => thumbs_cache_dir().join(fname),
                "wave" => waveforms_cache_dir().join(fname),
                "spec" => spectrograms_cache_dir().join(fname),
                _ => {
                    return Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(Vec::new())
                        .unwrap()
                }
            };

            let Ok(bytes) = fs::read(&file_path) else {
                return Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Vec::new())
                    .unwrap();
            };

            let mime = if fname.ends_with(".png") {
                "image/png"
            } else {
                "image/jpeg"
            };

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::CACHE_CONTROL, "public, max-age=86400")
                .body(bytes)
                .unwrap()
        })
        .manage(app_state)
        .setup(move |app| {
            #[cfg(windows)]
            app.manage(LaunchMode::App);
            let handle = app.handle().clone();

            // The interface learns the system here: macOS keeps its own window buttons on the left.
            let init_script = format!(
                "window.__BOLADAO_LAUNCH__ = {{\"mode\":\"app\"}}; window.__BOLADAO_OS__ = \"{}\";",
                std::env::consts::OS
            );
            let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(APP_NAME)
                .inner_size(1120.0, 740.0)
                .min_inner_size(920.0, 620.0)
                .shadow(true)
                .resizable(true)
                .center()
                .background_color(Color(14, 14, 16, 255))
                .initialization_script(&init_script);
            // Windows and Linux draw the title bar themselves; macOS keeps the native buttons over
            // the app's own top bar.
            #[cfg(not(target_os = "macos"))]
            let builder = builder.decorations(false);
            #[cfg(target_os = "macos")]
            let builder = builder
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                // Centered on the brand pill (32 px tall, 9 px from the top).
                .traffic_light_position(tauri::LogicalPosition::new(16.0, 19.0));
            builder.build()?;

            queue.bind_app_handle(handle.clone());

            setup_system_tray(&handle)?;

            let handle_instance = handle.clone();
            single_instance.listen(move || show_main_window(&handle_instance));

            // Background 1: Extract embedded binaries on first boot + probe FFmpeg capabilities + weekly silent yt-dlp check
            let bins_boot = bins.clone();
            let settings_boot = settings.clone();
            let handle_boot = handle.clone();
            tauri::async_runtime::spawn(async move {
                let channel = settings_boot.read().ytdlp_channel.clone();
                if let Err(e) = bins_boot.ensure_ready(Some(handle_boot.clone()), channel.clone()).await {
                    eprintln!("[boladao] Erro extraindo binários: {e}");
                    return;
                }

                // Silent weekly check for yt-dlp updates (Section 3.3)
                let need_weekly = settings_boot
                    .read()
                    .last_ytdlp_check
                    .map(|dt| (Utc::now() - dt).num_days() >= 7)
                    .unwrap_or(true);
                if need_weekly {
                    if let Ok(Some(new_ver)) = bins_boot.check_ytdlp_update(&channel).await {
                        let _ = handle_boot.emit("bins:update_available", new_ver);
                    }
                    let mut cfg = settings_boot.write();
                    cfg.last_ytdlp_check = Some(Utc::now());
                    let _ = save_settings_to_disk(&cfg);
                }
            });

            // Background 2: Limiter stats emitter at 4 Hz (250ms) + 7-day peak persistence
            let limiter_bg = limiter.clone();
            let settings_lim = settings.clone();
            let handle_lim = handle.clone();
            tauri::async_runtime::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_millis(250));
                let mut day = chrono::Local::now().date_naive();
                loop {
                    ticker.tick().await;
                    let today = chrono::Local::now().date_naive();
                    if today != day {
                        day = today;
                        let mut cfg = settings_lim.write();
                        cfg.peak_7d_bps = limiter::prune_daily_peaks(&mut cfg.daily_peaks, today);
                        limiter_bg.start_new_day(cfg.peak_7d_bps);
                        let _ = save_settings_to_disk(&cfg);
                    }
                    let (stats, new_peak) = limiter_bg.sample_tick_250ms();
                    if let Some(pk) = new_peak {
                        let mut cfg = settings_lim.write();
                        cfg.daily_peaks.insert(today.to_string(), pk);
                        cfg.peak_7d_bps = limiter::prune_daily_peaks(&mut cfg.daily_peaks, today);
                        let _ = save_settings_to_disk(&cfg);
                    }
                    let _ = handle_lim.emit("limiter:stats", stats);
                }
            });

            // Background 2b: Boladão self-update through GitHub Releases (shortly after start, then every 6 h)
            let limiter_upd = limiter.clone();
            let handle_upd = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(20)).await;
                loop {
                    match updater::check_and_download(limiter_upd.proxy_url()).await {
                        Ok(Some(update)) => {
                            let _ = handle_upd.emit(
                                "app:update_ready",
                                ipc::AppUpdateInfo {
                                    version: update.version,
                                    notes: update.notes,
                                },
                            );
                        }
                        Ok(None) => {}
                        Err(e) => eprintln!("[boladao] Verificação de atualização: {e}"),
                    }
                    tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
                }
            });

            // Background 3: Game Mode automatic detector every 2s (Section 11.4)
            let gm_bg = gamemode.clone();
            let limiter_gm = limiter.clone();
            let queue_gm = queue.clone();
            let settings_gm = settings.clone();
            let handle_gm = handle.clone();
            tauri::async_runtime::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(2));
                loop {
                    ticker.tick().await;
                    let (auto_en, ovr, exes, g_prof, g_bps) = {
                        let cfg = settings_gm.read();
                        (
                            cfg.game_mode_auto,
                            cfg.game_mode_override,
                            cfg.game_exe_list.clone(),
                            cfg.game_bandwidth_profile,
                            cfg.game_custom_limit_bps,
                        )
                    };
                    if let Some(changed) = gm_bg.poll_step(auto_en, ovr, &exes) {
                        limiter_gm.set_game_mode_active(changed.active, g_prof, g_bps);
                        queue_gm.wake_scheduler();
                        let _ = handle_gm.emit("gamemode:changed", changed);
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                if let WindowEvent::Destroyed = event {
                    if let Some(plat) = window.label().strip_prefix("login-") {
                        let _ = window
                            .app_handle()
                            .emit("accounts:login_window_closed", plat.to_string());
                    }
                }
                return;
            }
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    let state = window.state::<AppState>();
                    if state.queue.has_active_jobs() {
                        api.prevent_close();
                        let _ = window.hide();
                        let first_time = {
                            let mut cfg = state.settings.write();
                            if !cfg.tray_notice_shown {
                                cfg.tray_notice_shown = true;
                                let _ = save_settings_to_disk(&cfg);
                                true
                            } else {
                                false
                            }
                        };
                        if first_time {
                            let _ = window
                                .app_handle()
                                .notification()
                                .builder()
                                .title(APP_NAME)
                                .body(tr!(
                                    "Downloads em andamento: o Boladão ficou na bandeja do sistema.",
                                    "Downloads in progress: Boladão is still running in the system tray.",
                                    "Descargas en curso: Boladão sigue en la bandeja del sistema."
                                ))
                                .show();
                        }
                    }
                }
                WindowEvent::Destroyed => {
                    window.app_handle().exit(0);
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            #[cfg(windows)]
            ipc::get_launch_mode,
            ipc::get_system_language,
            #[cfg(windows)]
            ipc::get_installer_defaults,
            #[cfg(windows)]
            ipc::install_boladao,
            #[cfg(windows)]
            ipc::launch_installed_boladao,
            #[cfg(windows)]
            ipc::uninstall_boladao,
            #[cfg(windows)]
            ipc::close_uninstaller,
            ipc::get_bins_status,
            ipc::run_ytdlp_diagnostic,
            ipc::list_supported_sites,
            ipc::check_ytdlp_update,
            ipc::update_ytdlp,
            ipc::rollback_ytdlp,
            ipc::get_pending_update,
            ipc::apply_update_now,
            ipc::open_author_profile,
            ipc::classify_url_cmd,
            ipc::extract_urls_cmd,
            ipc::probe,
            ipc::preview_path_cmd,
            ipc::get_free_disk_space_cmd,
            ipc::enqueue,
            ipc::enqueue_batch,
            ipc::enqueue_playlist,
            ipc::get_queue,
            ipc::pause,
            ipc::resume,
            ipc::cancel,
            ipc::retry,
            ipc::remove_jobs_cmd,
            ipc::pause_group_cmd,
            ipc::resume_group_cmd,
            ipc::pause_all_cmd,
            ipc::resume_all_cmd,
            ipc::clear_completed_cmd,
            ipc::reorder,
            ipc::set_concurrency,
            ipc::set_limit,
            ipc::get_gamemode_status,
            ipc::set_gamemode_override,
            ipc::list_running_processes_cmd,
            ipc::equivalent_command,
            ipc::open_file,
            ipc::reveal_in_explorer_cmd,
            ipc::open_data_folder_cmd,
            ipc::start_file_drag,
            ipc::get_settings,
            ipc::save_settings,
            ipc::get_presets,
            ipc::save_custom_preset,
            ipc::delete_custom_preset,
            ipc::login_platform,
            ipc::finish_login_platform,
            ipc::connect_firefox_cookies,
            ipc::import_cookies_file_cmd,
            ipc::logout_platform,
            ipc::query_library,
            ipc::delete_library_entry,
            ipc::generate_spectrogram,
            ipc::run_workshop,
        ])
        .build(tauri::generate_context!())
        .expect("Erro executando aplicação Tauri")
        .run(|_app, _event| {
            // macOS: clicking the Dock icon brings back the window hidden in the menu bar.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                show_main_window(_app);
            }
        });
}

/// Cached media names as the app generates them: a SHA-256 hex or a UUID, plus `.img` or `.png`.
fn is_media_cache_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    matches!(ext, "img" | "png")
        && !stem.is_empty()
        && stem.len() <= 64
        && stem.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// Tray menu entries, kept so their labels can follow a language change.
struct TrayItems(Vec<MenuItem<tauri::Wry>>);

fn tray_label(id: &str) -> String {
    match id {
        "show" => tr!("Abrir Boladão", "Open Boladão", "Abrir Boladão"),
        "pause_all" => tr!("Pausar tudo", "Pause all", "Pausar todo"),
        "resume_all" => tr!("Retomar tudo", "Resume all", "Reanudar todo"),
        "bw_unlimited" => tr!(
            "Banda: Sem limite (∞)",
            "Bandwidth: Unlimited (∞)",
            "Ancho de banda: Sin límite (∞)"
        ),
        "bw_balanced" => tr!(
            "Banda: Equilibrado (60%)",
            "Bandwidth: Balanced (60%)",
            "Ancho de banda: Equilibrado (60%)"
        ),
        "bw_light" => tr!(
            "Banda: Leve (2 MB/s)",
            "Bandwidth: Light (2 MB/s)",
            "Ancho de banda: Ligero (2 MB/s)"
        ),
        "open_folder" => tr!("Abrir pasta de vídeos", "Open videos folder", "Abrir carpeta de videos"),
        "quit" => tr!("Sair", "Quit", "Salir"),
        _ => String::new(),
    }
}

/// Rewrites the tray labels in the current language.
pub fn refresh_tray_texts(app: &AppHandle) {
    if let Some(items) = app.try_state::<TrayItems>() {
        for item in &items.0 {
            let _ = item.set_text(tray_label(item.id().as_ref()));
        }
    }
}

/// Brings the main window back from the tray (or from minimized) and gives it focus.
fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn setup_system_tray(app: &AppHandle) -> tauri::Result<()> {
    let item = |id: &str| MenuItem::with_id(app, id, tray_label(id), true, None::<&str>);
    let show_i = item("show")?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let pause_i = item("pause_all")?;
    let resume_i = item("resume_all")?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let bw_inf = item("bw_unlimited")?;
    let bw_bal = item("bw_balanced")?;
    let bw_light = item("bw_light")?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let open_dir = item("open_folder")?;
    let quit_i = item("quit")?;
    app.manage(TrayItems(vec![
        show_i.clone(),
        pause_i.clone(),
        resume_i.clone(),
        bw_inf.clone(),
        bw_bal.clone(),
        bw_light.clone(),
        open_dir.clone(),
        quit_i.clone(),
    ]));

    let menu = Menu::with_items(
        app,
        &[
            &show_i, &sep1, &pause_i, &resume_i, &sep2, &bw_inf, &bw_bal, &bw_light, &sep3,
            &open_dir, &quit_i,
        ],
    )?;

    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip(APP_NAME)
        // macOS opens menu bar menus with a plain click; Windows keeps the click for the window.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| {
            let state = app.state::<AppState>();
            match event.id.as_ref() {
                "show" => show_main_window(app),
                "pause_all" => state.queue.pause_all(),
                "resume_all" => state.queue.resume_all(),
                "bw_unlimited" => {
                    state
                        .limiter
                        .apply_profile(BandwidthProfile::Unlimited, None, None);
                }
                "bw_balanced" => {
                    state
                        .limiter
                        .apply_profile(BandwidthProfile::Balanced, None, None);
                }
                "bw_light" => {
                    state
                        .limiter
                        .apply_profile(BandwidthProfile::Light, None, None);
                }
                "open_folder" => {
                    let root = state.settings.read().video_root.clone();
                    let _ = fs::create_dir_all(&root);
                    let _ = reveal_in_explorer(&root);
                }
                // Nothing is paused on the way out: the Job Objects stop the running processes, and
                // on the next start active items go back to the queue and resume their `.part`
                // files, while queued ones simply stay queued.
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_media_cache_name;

    #[test]
    fn media_protocol_serves_only_names_the_app_generates() {
        assert!(is_media_cache_name(&format!("{}.img", "a1".repeat(32))));
        assert!(is_media_cache_name("0f8fad5b-d9cb-469f-a165-70867728950e.png"));

        let too_long = format!("{}.img", "a".repeat(65));
        for bad in [
            "",
            ".png",
            "C:segredo.png",
            "D:a.png",
            "..\\settings.json",
            "a/b.png",
            "%2e%2e.png",
            "abc.exe",
            "abc.png:fluxo",
            "abc",
            too_long.as_str(),
        ] {
            assert!(!is_media_cache_name(bad), "{bad:?} não pode ser servido");
        }
    }
}
