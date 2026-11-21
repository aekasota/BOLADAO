pub mod args;
pub mod bins;
pub mod classify;
pub mod cookies;
pub mod errors;
pub mod gamemode;
pub mod installer;
pub mod ipc;
pub mod job;
pub mod library;
pub mod limiter;
pub mod paths;
pub mod probe;
pub mod queue;
pub mod settings;
pub mod spec;
pub mod winproc;
pub mod workshop;

use crate::bins::BinManager;
use crate::gamemode::GameModeMonitor;
use crate::installer::{detect_launch_mode, ensure_webview2_or_bootstrap, LaunchMode};
use crate::ipc::AppState;
use crate::library::{spectrograms_cache_dir, waveforms_cache_dir, LibraryStore};
use crate::limiter::{BandwidthLimiter, BandwidthProfile};
use crate::paths::{cleanup_stale_temp_dirs, reveal_in_explorer, APP_NAME};
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
                LaunchMode::Installer => ("Instalar Boladão", 760.0, 548.0),
                LaunchMode::Uninstaller { .. } => ("Desinstalar Boladão", 640.0, 440.0),
                LaunchMode::App => (APP_NAME, 1120.0, 740.0),
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
    if std::env::args().any(|arg| arg == "--setup-init") {
        run_setup_init();
        return;
    }

    let launch_mode = detect_launch_mode();
    if !matches!(launch_mode, LaunchMode::App) {
        run_installer_or_uninstaller_ui(launch_mode);
        return;
    }

    if !ensure_webview2_or_bootstrap() {
        winproc::show_missing_webview2_dialog();
        return;
    }

    let settings = Arc::new(RwLock::new(load_or_init_settings()));
    {
        let cfg = settings.read();
        let _ = fs::create_dir_all(&cfg.music_root);
        let _ = fs::create_dir_all(&cfg.video_root);
        cleanup_stale_temp_dirs(&[cfg.music_root.clone(), cfg.video_root.clone()]);
    }

    let bins = BinManager::new();
    let probe = Arc::new(ProbeService::new());
    let gamemode = GameModeMonitor::new();
    let library = Arc::new(
        LibraryStore::open_default().expect("Não foi possível inicializar banco SQLite da Biblioteca"),
    );

    let rt = tokio::runtime::Runtime::new().expect("Falha iniciando runtime Tokio");
    tauri::async_runtime::set(rt.handle().clone());

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
        .register_uri_scheme_protocol("boladaomedia", |_ctx, request| {
            let path = request.uri().path().trim_start_matches('/');
            let mut parts = path.splitn(2, '/');
            let kind = parts.next().unwrap_or("");
            let fname = parts.next().unwrap_or("");

            // Block any path traversal
            if fname.is_empty()
                || fname.contains("..")
                || fname.contains('/')
                || fname.contains('\\')
            {
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
        .manage(LaunchMode::App)
        .setup(move |app| {
            let handle = app.handle().clone();

            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(APP_NAME)
                .inner_size(1120.0, 740.0)
                .min_inner_size(920.0, 620.0)
                .decorations(false)
                .shadow(true)
                .resizable(true)
                .center()
                .background_color(Color(14, 14, 16, 255))
                .initialization_script("window.__BOLADAO_LAUNCH__ = {\"mode\":\"app\"};")
                .build()?;

            queue.bind_app_handle(handle.clone());

            setup_system_tray(&handle)?;

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
                loop {
                    ticker.tick().await;
                    let (stats, new_peak) = limiter_bg.sample_tick_250ms();
                    if let Some(pk) = new_peak {
                        let mut cfg = settings_lim.write();
                        cfg.peak_7d_bps = pk;
                        let _ = save_settings_to_disk(&cfg);
                    }
                    let _ = handle_lim.emit("limiter:stats", stats);
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
                                .body("Downloads em andamento: o Boladão ficou na bandeja do sistema.")
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
            ipc::get_launch_mode,
            ipc::get_installer_defaults,
            ipc::install_boladao,
            ipc::launch_installed_boladao,
            ipc::uninstall_boladao,
            ipc::close_uninstaller,
            ipc::get_bins_status,
            ipc::run_ytdlp_diagnostic,
            ipc::list_supported_sites,
            ipc::check_ytdlp_update,
            ipc::update_ytdlp,
            ipc::rollback_ytdlp,
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
        .run(tauri::generate_context!())
        .expect("Erro executando aplicação Tauri");
}

fn setup_system_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show", "Abrir Boladão", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let pause_i = MenuItem::with_id(app, "pause_all", "Pausar tudo", true, None::<&str>)?;
    let resume_i = MenuItem::with_id(app, "resume_all", "Retomar tudo", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let bw_inf = MenuItem::with_id(app, "bw_unlimited", "Banda: Sem limite (∞)", true, None::<&str>)?;
    let bw_bal = MenuItem::with_id(app, "bw_balanced", "Banda: Equilibrado (60%)", true, None::<&str>)?;
    let bw_light = MenuItem::with_id(app, "bw_light", "Banda: Leve (2 MB/s)", true, None::<&str>)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let open_dir = MenuItem::with_id(app, "open_folder", "Abrir pasta de vídeos", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;

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
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let state = app.state::<AppState>();
            match event.id.as_ref() {
                "show" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
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
                "quit" => {
                    state.queue.pause_all();
                    app.exit(0);
                }
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
                if let Some(w) = tray.app_handle().get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}
