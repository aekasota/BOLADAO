use crate::bins::{BinManager, BinsStatusPayload};
use crate::classify::{classify_url, extract_urls_from_text, Classification};
use crate::cookies::{
    harvest_login_window_cookies, import_cookies_txt_file, logout_account,
    open_isolated_login_window, prepare_cookie_source, AccountAuthKind, ConnectedAccount,
};
use crate::gamemode::{GameModeMonitor, GameModeOverride, GameModeStatus};
use crate::installer::{
    close_and_cleanup_temp_uninstaller, get_installer_defaults_info,
    launch_installed_app_and_close, run_install_pipeline, run_uninstall_pipeline,
    InstallerDefaults, LaunchMode,
};
use crate::job::Job;
use crate::library::{
    analyze_completed_file, generate_spectrogram_and_cutoff, LibraryEntry, LibraryQuery,
    LibraryStore,
};
use crate::limiter::{BandwidthLimiter, BandwidthProfile, LimiterStatsPayload};
use crate::paths::{
    app_config_dir, app_local_dir, free_disk_bytes, open_file_default, open_url_default,
    preview_output_path, reveal_in_explorer, PathPreviewSample,
};
use crate::probe::{PlaylistEntryPreview, ProbeResult, ProbeScope, ProbeService};
use crate::queue::{EnqueueOutcome, EnqueueRequest, QueueManager};
use crate::settings::{save_settings_to_disk, AppSettings};
use crate::spec::{builtin_presets, DownloadSpec, Preset};
use crate::winproc::{list_running_exe_names, JobGroup, ProcessPriority};
use crate::workshop::run_workshop_pipeline;
use chrono::Utc;
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, State, WebviewWindow};
use uuid::Uuid;

pub struct AppState {
    pub bins: Arc<BinManager>,
    pub probe: Arc<ProbeService>,
    pub limiter: Arc<BandwidthLimiter>,
    pub gamemode: Arc<GameModeMonitor>,
    pub library: Arc<LibraryStore>,
    pub queue: Arc<QueueManager>,
    pub settings: Arc<RwLock<AppSettings>>,
}

#[tauri::command]
pub fn get_bins_status(state: State<'_, AppState>) -> BinsStatusPayload {
    state.bins.status()
}

#[tauri::command]
pub async fn run_ytdlp_diagnostic(state: State<'_, AppState>) -> Result<String, String> {
    state.bins.run_diagnostic().await
}

#[tauri::command]
pub async fn list_supported_sites(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    state.bins.list_supported_sites().await
}

#[tauri::command]
pub async fn check_ytdlp_update(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let channel = state.settings.read().ytdlp_channel.clone();
    let res = state.bins.check_ytdlp_update(&channel).await?;
    {
        let mut cfg = state.settings.write();
        cfg.last_ytdlp_check = Some(Utc::now());
        let _ = save_settings_to_disk(&cfg);
    }
    Ok(res)
}

#[tauri::command]
pub async fn update_ytdlp(
    app: AppHandle,
    state: State<'_, AppState>,
    channel: Option<String>,
) -> Result<String, String> {
    let ch = channel.unwrap_or_else(|| state.settings.read().ytdlp_channel.clone());
    let new_ver = state.bins.update_ytdlp(&ch, Some(&app)).await?;
    {
        let mut cfg = state.settings.write();
        cfg.ytdlp_channel = ch;
        cfg.last_ytdlp_check = Some(Utc::now());
        let _ = save_settings_to_disk(&cfg);
    }
    Ok(new_ver)
}

#[tauri::command]
pub fn rollback_ytdlp(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    state.bins.rollback_ytdlp(Some(&app))
}

/// Windows interface language and regional format, used when the language setting is automatic.
#[derive(serde::Serialize, Clone)]
pub struct SystemLanguage {
    pub language: crate::i18n::Lang,
    pub locale: String,
}

#[tauri::command]
pub fn get_system_language() -> SystemLanguage {
    SystemLanguage {
        language: crate::i18n::system(),
        locale: crate::i18n::system_locale(),
    }
}

#[derive(serde::Serialize, Clone)]
pub struct AppUpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

/// Update already downloaded and verified, if any (the banner may mount after the event fired).
#[tauri::command]
pub fn get_pending_update() -> Option<AppUpdateInfo> {
    crate::updater::read_pending().map(|p| AppUpdateInfo {
        version: p.version,
        notes: p.notes,
    })
}

/// Installs the pending update and restarts. Running downloads are killed with the old process and
/// resume from their partial files after the restart.
#[tauri::command]
pub fn apply_update_now(app: AppHandle) -> Result<(), String> {
    if crate::updater::apply_pending(true)? {
        app.exit(0);
        Ok(())
    } else {
        Err(tr!("Nenhuma atualização pronta para instalar.", "No update ready to install.", "No hay ninguna actualización lista para instalar."))
    }
}

/// Opens the author's GitHub profile from the status bar. The URL is fixed here so the webview
/// cannot use this command to open arbitrary targets.
#[tauri::command]
pub fn open_author_profile() -> Result<(), String> {
    open_url_default("https://github.com/aekasota")
}

#[tauri::command]
pub fn classify_url_cmd(url: String) -> Result<Classification, String> {
    classify_url(&url)
}

#[tauri::command]
pub fn extract_urls_cmd(text: String) -> Vec<String> {
    extract_urls_from_text(&text)
}

#[tauri::command]
pub async fn probe(
    state: State<'_, AppState>,
    url: String,
    scope: Option<ProbeScope>,
) -> Result<ProbeResult, crate::errors::JobError> {
    let cls = classify_url(&url).map_err(|msg| crate::errors::JobError {
        code: "invalid_url".to_string(),
        message_pt: msg,
        hint_pt: None,
        action: crate::errors::ErrorAction::None,
        transient: false,
        suggest_impersonate: false,
    })?;
    let (cookies, cookie_lease) = {
        let cfg = state.settings.read();
        prepare_cookie_source(&cls.platform_id, &cfg.accounts)
    };
    let result = state
        .probe
        .probe_url(
            &url,
            scope.unwrap_or(ProbeScope::SingleVideo),
            &state.bins,
            state.limiter.proxy_url(),
            cookies,
        )
        .await;
    drop(cookie_lease);
    result
}

#[tauri::command]
pub fn preview_path_cmd(
    root_dir: PathBuf,
    template: String,
    sample: PathPreviewSample,
) -> String {
    preview_output_path(&root_dir, &template, &sample)
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
pub fn get_free_disk_space_cmd(path: PathBuf) -> Option<u64> {
    free_disk_bytes(&path)
}

#[tauri::command]
pub fn enqueue(state: State<'_, AppState>, req: EnqueueRequest) -> Result<EnqueueOutcome, String> {
    // Remember last choice per platform (Section 5)
    if let Ok(cls) = classify_url(&req.url) {
        let mut cfg = state.settings.write();
        cfg.last_spec_by_platform
            .insert(cls.platform_id, req.spec.clone());
        let root = req.spec.output_root.clone();
        if !cfg.recent_folders.contains(&root) {
            cfg.recent_folders.insert(0, root);
            cfg.recent_folders.truncate(8);
        }
        let _ = save_settings_to_disk(&cfg);
    }
    state.queue.enqueue_single(req)
}

#[tauri::command]
pub fn enqueue_batch(
    state: State<'_, AppState>,
    urls: Vec<String>,
    spec: DownloadSpec,
    high_priority: bool,
) -> Result<Vec<Job>, String> {
    let mut created = Vec::new();
    for u in urls {
        let outcome = state.queue.enqueue_single(EnqueueRequest {
            url: u,
            title: None,
            channel_or_artist: None,
            thumb_url: None,
            duration_s: None,
            estimated_bytes: None,
            extractor_key: None,
            spec: spec.clone(),
            high_priority,
            force_duplicate: true,
            group_id: None,
            group_title: None,
            playlist_index: None,
        })?;
        if let EnqueueOutcome::Enqueued { mut jobs } = outcome {
            created.append(&mut jobs);
        }
    }
    Ok(created)
}

#[tauri::command]
pub fn enqueue_playlist(
    state: State<'_, AppState>,
    playlist_title: String,
    platform_url: String,
    items: Vec<PlaylistEntryPreview>,
    spec: DownloadSpec,
    high_priority: bool,
) -> Result<Vec<Job>, String> {
    state
        .queue
        .enqueue_playlist_items(playlist_title, platform_url, items, spec, high_priority)
}

#[tauri::command]
pub fn get_queue(state: State<'_, AppState>) -> Vec<Job> {
    state.queue.snapshot()
}

#[tauri::command]
pub fn pause(state: State<'_, AppState>, id: Uuid) {
    state.queue.pause_job(id);
}

#[tauri::command]
pub fn resume(state: State<'_, AppState>, id: Uuid) {
    state.queue.resume_job(id);
}

#[tauri::command]
pub fn cancel(state: State<'_, AppState>, id: Uuid) {
    state.queue.cancel_job(id);
}

#[tauri::command]
pub fn retry(state: State<'_, AppState>, id: Uuid) {
    state.queue.retry_job(id);
}

#[tauri::command]
pub fn remove_jobs_cmd(state: State<'_, AppState>, ids: Vec<Uuid>) {
    state.queue.remove_jobs(&ids);
}

#[tauri::command]
pub fn pause_group_cmd(state: State<'_, AppState>, group_id: Uuid) {
    state.queue.pause_group(group_id);
}

#[tauri::command]
pub fn resume_group_cmd(state: State<'_, AppState>, group_id: Uuid) {
    state.queue.resume_group(group_id);
}

#[tauri::command]
pub fn pause_all_cmd(state: State<'_, AppState>) {
    state.queue.pause_all();
}

#[tauri::command]
pub fn resume_all_cmd(state: State<'_, AppState>) {
    state.queue.resume_all();
}

#[tauri::command]
pub fn clear_completed_cmd(state: State<'_, AppState>) {
    state.queue.clear_completed();
}

#[tauri::command]
pub fn reorder(state: State<'_, AppState>, ordered_ids: Vec<Uuid>) {
    state.queue.reorder_jobs(&ordered_ids);
}

#[tauri::command]
pub fn set_concurrency(state: State<'_, AppState>, concurrency: usize) -> Result<(), String> {
    {
        let mut cfg = state.settings.write();
        cfg.global_concurrency = concurrency.clamp(1, 6);
        save_settings_to_disk(&cfg)?;
    }
    state.queue.wake_scheduler();
    Ok(())
}

#[tauri::command]
pub fn set_limit(
    state: State<'_, AppState>,
    profile: BandwidthProfile,
    custom_bps: Option<u64>,
) -> Result<LimiterStatsPayload, String> {
    let light_bps = {
        let mut cfg = state.settings.write();
        cfg.bandwidth_profile = profile;
        if let Some(c) = custom_bps {
            cfg.custom_limit_bps = c;
        }
        save_settings_to_disk(&cfg)?;
        cfg.light_limit_bps
    };
    state
        .limiter
        .apply_profile(profile, custom_bps, Some(light_bps));
    let (stats, _) = state.limiter.sample_tick_250ms();
    Ok(stats)
}

#[tauri::command]
pub fn get_gamemode_status(state: State<'_, AppState>) -> GameModeStatus {
    state.gamemode.status()
}

#[tauri::command]
pub fn set_gamemode_override(
    state: State<'_, AppState>,
    override_mode: GameModeOverride,
) -> Result<GameModeStatus, String> {
    let (auto_en, exes, g_prof, g_bps) = {
        let mut cfg = state.settings.write();
        cfg.game_mode_override = override_mode;
        save_settings_to_disk(&cfg)?;
        (
            cfg.game_mode_auto,
            cfg.game_exe_list.clone(),
            cfg.game_bandwidth_profile,
            cfg.game_custom_limit_bps,
        )
    };
    if let Some(changed) = state.gamemode.poll_step(auto_en, override_mode, &exes) {
        state
            .limiter
            .set_game_mode_active(changed.active, g_prof, g_bps);
        state.queue.wake_scheduler();
        return Ok(changed);
    }
    Ok(state.gamemode.status())
}

#[tauri::command]
pub fn list_running_processes_cmd() -> Vec<String> {
    list_running_exe_names()
}

#[tauri::command]
pub fn equivalent_command(state: State<'_, AppState>, job_id: Uuid) -> Result<String, String> {
    if let Some(j) = state.queue.get_job(job_id) {
        if !j.equivalent_cmd.is_empty() {
            return Ok(j.equivalent_cmd);
        }
    }
    if let Some(entry) = state.library.get_by_id(job_id) {
        return Ok(entry.equivalent_cmd);
    }
    Err(tr!("Comando equivalente ainda não disponível.", "Equivalent command not available yet.", "El comando equivalente aún no está disponible."))
}

#[tauri::command]
pub fn open_file(path: PathBuf) -> Result<(), String> {
    open_file_default(&path)
}

#[tauri::command]
pub fn reveal_in_explorer_cmd(path: PathBuf) -> Result<(), String> {
    reveal_in_explorer(&path)
}

#[tauri::command]
pub fn open_data_folder_cmd(which: String) -> Result<(), String> {
    let dir = if which == "local" {
        app_local_dir()
    } else {
        app_config_dir()
    };
    reveal_in_explorer(&dir)
}

/// Starts a native Windows drag-and-drop operation out of the app window into Explorer / DAW / Video Editor!
#[tauri::command]
pub fn start_file_drag(window: WebviewWindow, path: PathBuf) -> Result<(), String> {
    if !path.exists() {
        return Err(tr!("Arquivo não encontrado para arrastar.", "File not found to drag.", "No se encontró el archivo para arrastrar."));
    }
    // Use tiny 1x1 valid PNG drag preview icon
    const DRAG_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8,
        0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    let item = drag::DragItem::Files(vec![path]);
    let image = drag::Image::Raw(DRAG_PNG.to_vec());
    let _ = drag::start_drag(
        &window,
        item,
        image,
        |_res, _pos| {},
        drag::Options::default(),
    );
    Ok(())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.settings.read().clone()
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    mut settings: AppSettings,
) -> Result<(), String> {
    {
        // Speed peaks are measured here; the window's copy of them may be older.
        let current = state.settings.read();
        settings.daily_peaks = current.daily_peaks.clone();
        settings.peak_7d_bps = current.peak_7d_bps;
    }
    save_settings_to_disk(&settings)?;
    if settings.language != state.settings.read().language {
        crate::i18n::apply(settings.language);
        crate::refresh_tray_texts(&app);
    }
    state.limiter.apply_profile(
        settings.bandwidth_profile,
        Some(settings.custom_limit_bps),
        Some(settings.light_limit_bps),
    );
    *state.settings.write() = settings;
    state.queue.wake_scheduler();
    Ok(())
}

#[tauri::command]
pub fn get_presets(state: State<'_, AppState>) -> Vec<Preset> {
    let cfg = state.settings.read();
    let mut all = builtin_presets(&cfg.music_root, &cfg.video_root);
    all.extend(cfg.custom_presets.clone());
    all
}

#[tauri::command]
pub fn save_custom_preset(
    state: State<'_, AppState>,
    name: String,
    description: String,
    spec: DownloadSpec,
) -> Result<Preset, String> {
    let preset = Preset {
        id: format!("custom-{}", Uuid::new_v4().simple()),
        name,
        description,
        builtin: false,
        spec,
    };
    let mut cfg = state.settings.write();
    cfg.custom_presets.push(preset.clone());
    save_settings_to_disk(&cfg)?;
    Ok(preset)
}

#[tauri::command]
pub fn delete_custom_preset(state: State<'_, AppState>, preset_id: String) -> Result<(), String> {
    let mut cfg = state.settings.write();
    cfg.custom_presets.retain(|p| p.id != preset_id);
    save_settings_to_disk(&cfg)
}

// Accounts & Cookies (Section 12)
// IMPORTANT: Must be `async fn` on Windows so WebviewWindowBuilder::build() and cookies_for_url()
// execute off the Win32 main UI thread and never deadlock the WebView2 message pump!
#[tauri::command]
pub async fn login_platform(app: AppHandle, platform_id: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || open_isolated_login_window(&app, &platform_id))
        .await
        .map_err(|e| tr!("Erro abrindo janela de login: {e}", "Error opening the sign-in window: {e}", "Error al abrir la ventana de inicio de sesión: {e}"))?
}

#[tauri::command]
pub async fn finish_login_platform(
    app: AppHandle,
    state: State<'_, AppState>,
    platform_id: String,
) -> Result<ConnectedAccount, String> {
    let acc = tokio::task::spawn_blocking(move || harvest_login_window_cookies(&app, &platform_id))
        .await
        .map_err(|e| tr!("Erro lendo cookies: {e}", "Error reading cookies: {e}", "Error al leer las cookies: {e}"))??;
    let mut cfg = state.settings.write();
    cfg.accounts.insert(acc.platform_id.clone(), acc.clone());
    save_settings_to_disk(&cfg)?;
    Ok(acc)
}

#[tauri::command]
pub fn connect_firefox_cookies(
    state: State<'_, AppState>,
    platform_id: String,
) -> Result<ConnectedAccount, String> {
    let key = if platform_id == "youtube_music" {
        "youtube".to_string()
    } else {
        platform_id
    };
    let acc = ConnectedAccount {
        platform_id: key.clone(),
        auth_kind: AccountAuthKind::FirefoxBrowser,
        connected_at: Utc::now(),
        earliest_expiry_ts: None,
        cookie_count: 0,
    };
    let mut cfg = state.settings.write();
    cfg.accounts.insert(key, acc.clone());
    save_settings_to_disk(&cfg)?;
    Ok(acc)
}

#[tauri::command]
pub fn import_cookies_file_cmd(
    state: State<'_, AppState>,
    platform_id: String,
    file_path: PathBuf,
) -> Result<ConnectedAccount, String> {
    let acc = import_cookies_txt_file(&platform_id, &file_path)?;
    let mut cfg = state.settings.write();
    cfg.accounts.insert(acc.platform_id.clone(), acc.clone());
    save_settings_to_disk(&cfg)?;
    Ok(acc)
}

#[tauri::command]
pub async fn logout_platform(
    app: AppHandle,
    state: State<'_, AppState>,
    platform_id: String,
) -> Result<(), String> {
    let key = if platform_id == "youtube_music" {
        "youtube".to_string()
    } else {
        platform_id
    };
    let key_for_close = key.clone();
    let _ = tokio::task::spawn_blocking(move || {
        logout_account(Some(&app), &key_for_close);
    })
    .await;
    let mut cfg = state.settings.write();
    cfg.accounts.remove(&key);
    save_settings_to_disk(&cfg)
}

// Library & Standalone Workshop (Sections 8 & 14.4)
#[tauri::command]
pub fn query_library(
    state: State<'_, AppState>,
    query: LibraryQuery,
) -> Result<Vec<LibraryEntry>, String> {
    state.library.query(&query)
}

#[tauri::command]
pub fn delete_library_entry(
    state: State<'_, AppState>,
    id: Uuid,
    delete_disk_file: bool,
) -> Result<(), String> {
    state.library.delete_entry(id, delete_disk_file)
}

#[tauri::command]
pub async fn generate_spectrogram(
    state: State<'_, AppState>,
    id: Uuid,
) -> Result<LibraryEntry, String> {
    let mut entry = state
        .library
        .get_by_id(id)
        .ok_or_else(|| tr!("Item não encontrado na Biblioteca.", "Item not found in the Library.", "Elemento no encontrado en la Biblioteca."))?;
    let (spec_url, cutoff) =
        generate_spectrogram_and_cutoff(id, &entry.primary_path, &state.bins).await?;
    entry.spectrogram_url = Some(spec_url);
    entry.details.lossy_cutoff_note = cutoff;
    state.library.insert_or_update(&entry)?;
    Ok(entry)
}

#[tauri::command]
pub async fn run_workshop(
    state: State<'_, AppState>,
    entry_id: Uuid,
    spec: DownloadSpec,
) -> Result<LibraryEntry, String> {
    let mut entry = state
        .library
        .get_by_id(entry_id)
        .ok_or_else(|| tr!("Item não encontrado na Biblioteca.", "Item not found in the Library.", "Elemento no encontrado en la Biblioteca."))?;
    if !entry.primary_path.is_file() {
        return Err(tr!("O arquivo original não está mais no disco.", "The original file is no longer on disk.", "El archivo original ya no está en el disco."));
    }
    let jg = JobGroup::new().map_err(|e| e.to_string())?;
    let prio = if state.gamemode.status().active {
        ProcessPriority::Idle
    } else {
        ProcessPriority::BelowNormal
    };

    let outputs = run_workshop_pipeline(
        &jg,
        &state.bins,
        entry.primary_path.clone(),
        &spec,
        prio,
        |_st, _p| {},
    )
    .await?;

    if let Some(first) = outputs.first() {
        entry.primary_path = first.clone();
        entry.extra_paths = outputs.iter().skip(1).cloned().collect();
        let (details, wave) =
            analyze_completed_file(entry.id, &entry.primary_path, entry.mode, &state.bins).await;
        entry.details = details;
        if wave.is_some() {
            entry.waveform_url = wave;
        }
        entry.spectrogram_url = None;
        entry.file_exists = entry.primary_path.is_file();
        state.library.insert_or_update(&entry)?;
    }
    Ok(entry)
}

// Custom Op-Art Installer & Uninstaller IPC Commands
#[tauri::command]
pub fn get_launch_mode(mode: State<'_, LaunchMode>) -> LaunchMode {
    (*mode).clone()
}

#[tauri::command]
pub fn get_installer_defaults() -> InstallerDefaults {
    get_installer_defaults_info()
}

#[tauri::command]
pub async fn install_boladao(
    app: AppHandle,
    install_dir: PathBuf,
    create_desktop_shortcut: bool,
    create_start_menu_shortcut: bool,
) -> Result<PathBuf, String> {
    run_install_pipeline(
        app,
        install_dir,
        create_desktop_shortcut,
        create_start_menu_shortcut,
    )
    .await
}

#[tauri::command]
pub async fn launch_installed_boladao(
    app: AppHandle,
    installed_exe: PathBuf,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || launch_installed_app_and_close(&app, &installed_exe))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn uninstall_boladao(
    target_dir: PathBuf,
    remove_user_data: bool,
) -> Result<(), String> {
    run_uninstall_pipeline(target_dir, remove_user_data).await
}

#[tauri::command]
pub async fn close_uninstaller(app: AppHandle) -> Result<(), String> {
    tokio::task::spawn_blocking(move || close_and_cleanup_temp_uninstaller(&app))
        .await
        .map_err(|e| e.to_string())
}
