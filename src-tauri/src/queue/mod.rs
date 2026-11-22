use crate::args::ExecContext;
use crate::bins::BinManager;
use crate::classify::{classify_url, normalize_url_for_dedup, SourceKind};
use crate::cookies::prepare_cookie_source;
use crate::gamemode::GameModeMonitor;
use crate::job::{execute_job_pipeline, ActiveJobHandle, Job, JobState, Progress, SourceMeta};
use crate::library::{analyze_completed_file, LibraryEntry, LibraryStore};
use crate::limiter::BandwidthLimiter;
use crate::paths::{app_config_dir, cleanup_stale_temp_dirs, TEMP_SUBDIR_NAME};
use crate::probe::PlaylistEntryPreview;
use crate::settings::AppSettings;
use crate::spec::DownloadSpec;
use crate::winproc::{JobGroup, ProcessPriority};
use chrono::Utc;
use parking_lot::{Mutex, RwLock};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnqueueRequest {
    pub url: String,
    pub title: Option<String>,
    pub channel_or_artist: Option<String>,
    pub thumb_url: Option<String>,
    pub duration_s: Option<f64>,
    pub estimated_bytes: Option<u64>,
    pub extractor_key: Option<String>,
    pub spec: DownloadSpec,
    pub high_priority: bool, // Ctrl+Enter = Baixar agora (top of queue)
    pub force_duplicate: bool,
    pub group_id: Option<Uuid>,
    pub group_title: Option<String>,
    pub playlist_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EnqueueOutcome {
    Enqueued { jobs: Vec<Job> },
    AlreadyQueued { existing_id: Uuid },
    AlreadyInLibrary { entry: LibraryEntry },
}

pub struct QueueManager {
    db: Mutex<Connection>,
    jobs: RwLock<Vec<Job>>,
    active_handles: Mutex<HashMap<Uuid, Arc<ActiveJobHandle>>>,
    group_sizes: RwLock<HashMap<Uuid, usize>>,
    scheduler_notify: Notify,
    bins: Arc<BinManager>,
    limiter: Arc<BandwidthLimiter>,
    gamemode: Arc<GameModeMonitor>,
    library: Arc<LibraryStore>,
    settings: Arc<RwLock<AppSettings>>,
    app_handle: RwLock<Option<AppHandle>>,
    completed_since_notify: Mutex<(usize, Instant)>,
}

impl QueueManager {
    pub fn open_default(
        bins: Arc<BinManager>,
        limiter: Arc<BandwidthLimiter>,
        gamemode: Arc<GameModeMonitor>,
        library: Arc<LibraryStore>,
        settings: Arc<RwLock<AppSettings>>,
    ) -> Result<Arc<Self>, String> {
        let db_path = app_config_dir().join("boladao.db");
        let conn = Connection::open(&db_path).map_err(|e| format!("Erro abrindo fila SQLite: {e}"))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS queue_jobs (
                id TEXT PRIMARY KEY,
                priority INTEGER NOT NULL,
                created_at_ts INTEGER NOT NULL,
                state TEXT NOT NULL,
                payload_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_queue_order ON queue_jobs(priority DESC, created_at_ts ASC);",
        )
        .map_err(|e| format!("Erro inicializando tabela queue_jobs: {e}"))?;

        let mut loaded_jobs = load_jobs_from_db(&conn);
        // Section 9.1: Jobs that were active when the app closed return as `Queued` and resume `.part`!
        for j in &mut loaded_jobs {
            if j.state.is_active() {
                j.state = JobState::Queued;
                j.progress.stage = tr!("Na fila (retomando)", "Queued (resuming)", "En cola (reanudando)");
                j.progress.speed_bps = 0;
                j.progress.eta_s = None;
                let _ = persist_job_row(&conn, j);
            }
        }

        // `.boladao-tmp` holds the partial files that let interrupted downloads resume, so it is
        // only wiped for destination folders where no job can still use it.
        {
            let cfg = settings.read();
            let idle = idle_temp_roots(
                loaded_jobs.iter().map(|j| (j.spec.output_root.as_path(), j.state)),
                [cfg.music_root.as_path(), cfg.video_root.as_path()],
            );
            cleanup_stale_temp_dirs(&idle);
        }

        let mut group_sizes: HashMap<Uuid, usize> = HashMap::new();
        for j in &loaded_jobs {
            if let Some(gid) = j.group_id {
                *group_sizes.entry(gid).or_insert(0) += 1;
            }
        }

        let mgr = Arc::new(Self {
            db: Mutex::new(conn),
            jobs: RwLock::new(loaded_jobs),
            active_handles: Mutex::new(HashMap::new()),
            group_sizes: RwLock::new(group_sizes),
            scheduler_notify: Notify::new(),
            bins,
            limiter,
            gamemode,
            library,
            settings,
            app_handle: RwLock::new(None),
            completed_since_notify: Mutex::new((0, Instant::now())),
        });

        let mgr_bg = mgr.clone();
        tokio::spawn(async move {
            mgr_bg.scheduler_loop().await;
        });

        Ok(mgr)
    }

    pub fn bind_app_handle(&self, app: AppHandle) {
        *self.app_handle.write() = Some(app);
        self.wake_scheduler();
    }

    pub fn wake_scheduler(&self) {
        self.scheduler_notify.notify_one();
    }

    pub fn snapshot(&self) -> Vec<Job> {
        self.jobs.read().clone()
    }

    pub fn has_active_jobs(&self) -> bool {
        self.jobs.read().iter().any(|j| j.state.is_active())
    }

    pub fn get_job(&self, id: Uuid) -> Option<Job> {
        self.jobs.read().iter().find(|j| j.id == id).cloned()
    }

    pub fn enqueue_single(&self, req: EnqueueRequest) -> Result<EnqueueOutcome, String> {
        let cls = classify_url(&req.url)?;
        if cls.is_drm {
            return Err(cls
                .drm_message
                .unwrap_or_else(|| {
                tr!(
                    "Esse serviço usa DRM. O yt-dlp não baixa daqui.",
                    "This service uses DRM. yt-dlp can't download from it.",
                    "Este servicio usa DRM. yt-dlp no puede descargar de aquí."
                )
            }));
        }

        let dedup_key = normalize_url_for_dedup(&req.url).unwrap_or_else(|| cls.normalized_url.clone());

        // 1. Check if identical URL + mode is already active or queued
        if !req.force_duplicate {
            if let Some(existing) = self.jobs.read().iter().find(|j| {
                j.dedup_key == dedup_key
                    && j.spec.mode == req.spec.mode
                    && !j.state.is_terminal()
            }) {
                return Ok(EnqueueOutcome::AlreadyQueued {
                    existing_id: existing.id,
                });
            }

            if let Some(lib_hit) = self.library.has_downloaded_before(&dedup_key) {
                if lib_hit.file_exists && lib_hit.mode == req.spec.mode {
                    return Ok(EnqueueOutcome::AlreadyInLibrary { entry: lib_hit });
                }
            }
        }

        let priority = if req.high_priority {
            self.max_priority() + 10
        } else {
            self.min_priority() - 1
        };

        let job = Job {
            id: Uuid::new_v4(),
            url: cls.normalized_url.clone(),
            dedup_key,
            source: SourceMeta {
                platform_id: cls.platform_id.clone(),
                platform_name: cls.platform_name.clone(),
                extractor_key: req
                    .extractor_key
                    .unwrap_or_else(|| cls.platform_name.clone()),
                kind: cls.kind,
                brand_color: cls.brand_color,
                icon_slug: cls.icon_slug,
            },
            group_id: req.group_id,
            group_title: req.group_title,
            playlist_index: req.playlist_index,
            title: req.title.unwrap_or_else(|| cls.normalized_url.clone()),
            channel_or_artist: req
                .channel_or_artist
                .unwrap_or_else(|| cls.platform_name.clone()),
            thumb_url: req.thumb_url,
            duration_s: req.duration_s,
            estimated_bytes: req.estimated_bytes,
            spec: req.spec,
            state: JobState::Queued,
            progress: Progress::default(),
            outputs: Vec::new(),
            error: None,
            attempts: 0,
            priority,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            log_tail: VecDeque::new(),
            equivalent_cmd: String::new(),
        };

        {
            let conn = self.db.lock();
            persist_job_row(&conn, &job)?;
        }
        {
            let mut list = self.jobs.write();
            if req.high_priority {
                list.insert(0, job.clone());
            } else {
                list.push(job.clone());
            }
        }
        if let Some(gid) = job.group_id {
            *self.group_sizes.write().entry(gid).or_insert(0) += 1;
        }

        self.emit_queue_changed();
        self.wake_scheduler();
        Ok(EnqueueOutcome::Enqueued { jobs: vec![job] })
    }

    /// Enqueues selected playlist/channel items as a grouped batch (`group_id` shared across all N items).
    pub fn enqueue_playlist_items(
        &self,
        playlist_title: String,
        platform_url: String,
        items: Vec<PlaylistEntryPreview>,
        mut base_spec: DownloadSpec,
        high_priority: bool,
    ) -> Result<Vec<Job>, String> {
        if items.is_empty() {
            return Err(tr!("Nenhum item selecionado na playlist.", "No playlist items selected.", "No hay elementos seleccionados en la playlist."));
        }
        let base_cls = classify_url(&platform_url)?;
        let gid = Uuid::new_v4();
        base_spec.use_download_archive = true;

        self.group_sizes.write().insert(gid, items.len());

        let mut created = Vec::with_capacity(items.len());
        let base_prio = if high_priority {
            self.max_priority() + (items.len() as i32) + 10
        } else {
            self.min_priority() - 1
        };

        let conn = self.db.lock();
        let mut list = self.jobs.write();

        for (offset, item) in items.into_iter().enumerate() {
            let item_cls = classify_url(&item.url).unwrap_or_else(|_| base_cls.clone());
            let dedup = normalize_url_for_dedup(&item.url).unwrap_or_else(|| item.url.clone());
            let prio = base_prio - (offset as i32);

            let job = Job {
                id: Uuid::new_v4(),
                url: item.url,
                dedup_key: dedup,
                source: SourceMeta {
                    platform_id: item_cls.platform_id.clone(),
                    platform_name: item_cls.platform_name.clone(),
                    extractor_key: item_cls.platform_name.clone(),
                    kind: SourceKind::PlaylistItem,
                    brand_color: item_cls.brand_color,
                    icon_slug: item_cls.icon_slug,
                },
                group_id: Some(gid),
                group_title: Some(playlist_title.clone()),
                playlist_index: Some(item.index),
                title: item.title,
                channel_or_artist: item
                    .uploader
                    .unwrap_or_else(|| item_cls.platform_name.clone()),
                thumb_url: item.thumb_url,
                duration_s: item.duration_s,
                estimated_bytes: None,
                spec: base_spec.clone(),
                state: JobState::Queued,
                progress: Progress::default(),
                outputs: Vec::new(),
                error: None,
                attempts: 0,
                priority: prio,
                created_at: Utc::now(),
                started_at: None,
                finished_at: None,
                log_tail: VecDeque::new(),
                equivalent_cmd: String::new(),
            };
            persist_job_row(&conn, &job)?;
            list.push(job.clone());
            created.push(job);
        }
        drop(list);
        drop(conn);

        self.emit_queue_changed();
        self.wake_scheduler();
        Ok(created)
    }

    pub fn pause_job(&self, id: Uuid) {
        if let Some(h) = self.active_handles.lock().get(&id).cloned() {
            h.paused_flag.store(true, Ordering::SeqCst);
            h.job_group.terminate();
            return;
        }
        self.mutate_job(id, |j| {
            if j.state == JobState::Queued {
                j.state = JobState::Paused;
                j.progress.stage = "Pausado".to_string();
            }
        });
    }

    pub fn resume_job(&self, id: Uuid) {
        self.mutate_job(id, |j| {
            if matches!(j.state, JobState::Paused | JobState::Failed) {
                j.state = JobState::Queued;
                j.error = None;
                j.progress.stage = tr!("Na fila", "Queued", "En cola");
            }
        });
        self.wake_scheduler();
    }

    pub fn cancel_job(&self, id: Uuid) {
        let mut root_to_clean = None;
        let mut item_id_hint = None;
        if let Some(j) = self.get_job(id) {
            root_to_clean = Some(j.spec.output_root.clone());
            item_id_hint = extract_media_id_hint(&j.url);
        }

        if let Some(h) = self.active_handles.lock().get(&id).cloned() {
            h.canceled_flag.store(true, Ordering::SeqCst);
            h.job_group.terminate();
        } else {
            self.mutate_job(id, |j| {
                if !j.state.is_terminal() {
                    j.state = JobState::Canceled;
                    j.progress.stage = "Cancelado".to_string();
                    j.progress.speed_bps = 0;
                    j.finished_at = Some(Utc::now());
                }
            });
        }

        // Clean the canceled job's temporary files without touching partial downloads of other jobs
        // (paused or queued jobs are not "active", so a folder-wide wipe would destroy their progress).
        if let Some(root) = root_to_clean {
            if let Some(hint) = item_id_hint.as_deref() {
                remove_temp_files_with_id(&root.join(TEMP_SUBDIR_NAME), hint, 6);
            }
            self.clean_temp_if_idle(&root, Some(id));
        }
    }

    /// Wipes `<root>\.boladao-tmp` when no remaining job (besides `ignore`) can still resume from it.
    fn clean_temp_if_idle(&self, root: &Path, ignore: Option<Uuid>) {
        let busy = self
            .jobs
            .read()
            .iter()
            .any(|j| Some(j.id) != ignore && j.state.keeps_partial_files() && j.spec.output_root == root);
        if !busy {
            cleanup_stale_temp_dirs(&[root.to_path_buf()]);
        }
    }

    pub fn retry_job(&self, id: Uuid) {
        self.mutate_job(id, |j| {
            j.state = JobState::Queued;
            j.attempts = 0;
            j.error = None;
            j.progress.stage = tr!("Na fila", "Queued", "En cola");
            j.progress.speed_bps = 0;
            j.progress.eta_s = None;
        });
        self.wake_scheduler();
    }

    pub fn pause_group(&self, group_id: Uuid) {
        let ids: Vec<Uuid> = self
            .jobs
            .read()
            .iter()
            .filter(|j| j.group_id == Some(group_id))
            .map(|j| j.id)
            .collect();
        for id in ids {
            self.pause_job(id);
        }
    }

    pub fn resume_group(&self, group_id: Uuid) {
        let ids: Vec<Uuid> = self
            .jobs
            .read()
            .iter()
            .filter(|j| j.group_id == Some(group_id))
            .map(|j| j.id)
            .collect();
        for id in ids {
            self.resume_job(id);
        }
    }

    pub fn pause_all(&self) {
        let ids: Vec<Uuid> = self
            .jobs
            .read()
            .iter()
            .filter(|j| j.state.is_active() || j.state == JobState::Queued)
            .map(|j| j.id)
            .collect();
        for id in ids {
            self.pause_job(id);
        }
    }

    pub fn resume_all(&self) {
        let ids: Vec<Uuid> = self
            .jobs
            .read()
            .iter()
            .filter(|j| j.state == JobState::Paused)
            .map(|j| j.id)
            .collect();
        for id in ids {
            self.resume_job(id);
        }
    }

    pub fn remove_jobs(&self, ids: &[Uuid]) {
        for &id in ids {
            if let Some(h) = self.active_handles.lock().get(&id).cloned() {
                h.canceled_flag.store(true, Ordering::SeqCst);
                h.job_group.terminate();
            }
        }
        {
            let conn = self.db.lock();
            for id in ids {
                let _ = conn.execute(
                    "DELETE FROM queue_jobs WHERE id = ?1",
                    params![id.to_string()],
                );
            }
        }
        let touched_roots: BTreeSet<PathBuf> = self
            .jobs
            .read()
            .iter()
            .filter(|j| ids.contains(&j.id))
            .map(|j| j.spec.output_root.clone())
            .collect();
        self.jobs.write().retain(|j| !ids.contains(&j.id));
        for root in &touched_roots {
            self.clean_temp_if_idle(root, None);
        }
        self.emit_queue_changed();
        self.update_taskbar_progress();
    }

    pub fn clear_completed(&self) {
        let done_ids: Vec<Uuid> = self
            .jobs
            .read()
            .iter()
            .filter(|j| matches!(j.state, JobState::Done | JobState::Canceled))
            .map(|j| j.id)
            .collect();
        self.remove_jobs(&done_ids);
    }

    pub fn reorder_jobs(&self, ordered_ids: &[Uuid]) {
        let conn = self.db.lock();
        let mut list = self.jobs.write();
        let top_base = (ordered_ids.len() as i32) + 100;
        for (idx, id) in ordered_ids.iter().enumerate() {
            if let Some(j) = list.iter_mut().find(|x| &x.id == id) {
                j.priority = top_base - (idx as i32);
                let _ = persist_job_row(&conn, j);
            }
        }
        list.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        drop(list);
        drop(conn);
        self.emit_queue_changed();
        self.wake_scheduler();
    }

    async fn scheduler_loop(self: Arc<Self>) {
        loop {
            // Wait until binaries are ready
            if !self.bins.status().ready {
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }

            let spawned_any = self.try_dispatch_next();
            if !spawned_any {
                tokio::select! {
                    _ = self.scheduler_notify.notified() => {}
                    _ = tokio::time::sleep(Duration::from_millis(600)) => {}
                }
            }
        }
    }

    fn try_dispatch_next(self: &Arc<Self>) -> bool {
        let cfg = self.settings.read().clone();
        let gm_active = self.gamemode.status().active;

        let max_global = if gm_active {
            cfg.game_concurrency.clamp(1, 6)
        } else {
            cfg.global_concurrency.clamp(1, 6)
        };

        let mut list = self.jobs.write();
        let active_count = list.iter().filter(|j| j.state.is_active()).count();
        if active_count >= max_global {
            return false;
        }

        let mut active_by_host: HashMap<String, usize> = HashMap::new();
        for j in list.iter().filter(|j| j.state.is_active()) {
            let host_bucket = normalize_host_bucket(&j.source.platform_id);
            *active_by_host.entry(host_bucket).or_insert(0) += 1;
        }

        // Sort candidates by priority DESC, created_at ASC
        let candidate_idx = list
            .iter()
            .enumerate()
            .filter(|(_, j)| j.state == JobState::Queued)
            .max_by(|(_, a), (_, b)| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| b.created_at.cmp(&a.created_at))
            })
            .and_then(|(idx, candidate)| {
                let bucket = normalize_host_bucket(&candidate.source.platform_id);
                let host_cap = cfg.host_concurrency.get(&bucket).copied().unwrap_or(2);
                if active_by_host.get(&bucket).copied().unwrap_or(0) < host_cap {
                    Some(idx)
                } else {
                    None
                }
            });

        // If highest priority item is blocked by per-host cap, scan remaining Queued items for another host!
        let chosen_idx = candidate_idx.or_else(|| {
            list.iter().position(|j| {
                if j.state != JobState::Queued {
                    return false;
                }
                let bucket = normalize_host_bucket(&j.source.platform_id);
                let host_cap = cfg.host_concurrency.get(&bucket).copied().unwrap_or(2);
                active_by_host.get(&bucket).copied().unwrap_or(0) < host_cap
            })
        });

        let Some(idx) = chosen_idx else {
            return false;
        };

        list[idx].state = JobState::Downloading;
        list[idx].attempts = list[idx].attempts.saturating_add(1);
        list[idx].progress.stage = "Iniciando...".to_string();
        let job_to_run = list[idx].clone();
        {
            let conn = self.db.lock();
            let _ = persist_job_row(&conn, &job_to_run);
        }
        drop(list);

        let Ok(job_group) = JobGroup::new() else {
            return false;
        };
        let handle = Arc::new(ActiveJobHandle {
            job_group: Arc::new(job_group),
            paused_flag: Arc::new(AtomicBool::new(false)),
            canceled_flag: Arc::new(AtomicBool::new(false)),
        });
        self.active_handles
            .lock()
            .insert(job_to_run.id, handle.clone());
        self.emit_queue_changed();

        let mgr = self.clone();
        tokio::spawn(async move {
            mgr.run_single_job_task(job_to_run, handle).await;
        });

        true
    }

    async fn run_single_job_task(self: Arc<Self>, job: Job, handle: Arc<ActiveJobHandle>) {
        let job_id = job.id;
        let cfg = self.settings.read().clone();
        let gm_active = self.gamemode.status().active;

        let large_playlist = job
            .group_id
            .and_then(|gid| self.group_sizes.read().get(&gid).copied())
            .unwrap_or(1)
            > 30;

        let fallback_bps = if self.limiter.is_fallback_mode() {
            let total = self.limiter.current_limit_bps();
            if total > 0 {
                Some(total / (cfg.global_concurrency.max(1) as u64))
            } else {
                None
            }
        } else {
            None
        };

        let archive_path = Some(job.spec.output_root.join(".boladao-archive.txt"));
        let mut spec_with_globals = job.spec.clone();
        for g_arg in &cfg.global_extra_ytdlp_args {
            if !spec_with_globals.extra_ytdlp_args.contains(g_arg) {
                spec_with_globals.extra_ytdlp_args.push(g_arg.clone());
            }
        }
        let mut job_with_globals = job;
        job_with_globals.spec = spec_with_globals;

        // The lease holds the decrypted cookie copy for yt-dlp; it is dropped right after the pipeline.
        let (cookie_source, cookie_lease) =
            prepare_cookie_source(&job_with_globals.source.platform_id, &cfg.accounts);
        let ctx = ExecContext {
            ffmpeg_dir: self.bins.ffmpeg_dir(),
            deno_exe: self.bins.deno_exe(),
            proxy_url: self.limiter.proxy_url(),
            fallback_limit_bps: fallback_bps,
            cookies: cookie_source,
            archive_file: archive_path,
            apply_anti_429_sleep: large_playlist,
            impersonate_target: None,
            playlist_index_override: job_with_globals.playlist_index,
        };

        let ws_prio = if gm_active {
            ProcessPriority::Idle
        } else {
            ProcessPriority::BelowNormal
        };

        let mgr_progress = self.clone();
        let finished_job = execute_job_pipeline(
            job_with_globals,
            self.bins.clone(),
            ctx,
            cfg.collision_policy,
            ws_prio,
            handle,
            |live_job| {
                mgr_progress.sync_live_progress(live_job);
            },
        )
        .await;
        drop(cookie_lease);

        self.active_handles.lock().remove(&job_id);

        // Automatic exponential backoff retry (2s, 8s, 30s; max 3 retries) for transient errors (Section 9.2)
        if finished_job.state == JobState::Failed {
            let is_transient = finished_job
                .error
                .as_ref()
                .map(|e| e.transient)
                .unwrap_or(false);
            if is_transient && finished_job.attempts <= 3 {
                let delay_s = match finished_job.attempts {
                    1 => 2,
                    2 => 8,
                    _ => 30,
                };
                let mut retry_job = finished_job.clone();
                retry_job.progress.stage = tr!("Tentando de novo em {delay_s} s...", "Retrying in {delay_s} s...", "Reintentando en {delay_s} s...");
                self.replace_and_persist(&retry_job);
                self.emit_queue_changed();

                tokio::time::sleep(Duration::from_secs(delay_s)).await;
                // Re-queue if user hasn't canceled/paused meanwhile
                let mut should_wake = false;
                self.mutate_job(job_id, |j| {
                    if j.state == JobState::Failed {
                        j.state = JobState::Queued;
                        j.error = None;
                        j.progress.stage = "Retomando...".to_string();
                        should_wake = true;
                    }
                });
                if should_wake {
                    self.wake_scheduler();
                }
                return;
            }
        }

        // Persist final state
        self.replace_and_persist(&finished_job);

        if finished_job.state == JobState::Done {
            if let Some(primary) = finished_job.outputs.first().cloned() {
                let extra: Vec<_> = finished_job.outputs.iter().skip(1).cloned().collect();
                let entry_id = finished_job.id;
                let (details, waveform_url) = analyze_completed_file(
                    entry_id,
                    &primary,
                    finished_job.spec.mode,
                    &self.bins,
                )
                .await;

                let entry = LibraryEntry {
                    id: entry_id,
                    url: finished_job.url.clone(),
                    dedup_key: finished_job.dedup_key.clone(),
                    platform_id: finished_job.source.platform_id.clone(),
                    platform_name: finished_job.source.platform_name.clone(),
                    title: finished_job.title.clone(),
                    channel_or_artist: finished_job.channel_or_artist.clone(),
                    thumb_url: finished_job.thumb_url.clone(),
                    is_playlist_origin: finished_job.group_id.is_some(),
                    playlist_url: None,
                    mode: finished_job.spec.mode,
                    format_badge: finished_job.format_badge(),
                    primary_path: primary,
                    extra_paths: extra,
                    file_exists: true,
                    waveform_url,
                    spectrogram_url: None,
                    details,
                    equivalent_cmd: finished_job.equivalent_cmd.clone(),
                    spec: finished_job.spec.clone(),
                    created_at: Utc::now(),
                };
                let _ = self.library.insert_or_update(&entry);
            }

            if let Some(app) = self.app_handle.read().clone() {
                let _ = app.emit("job:completed", &finished_job.id);
                self.notify_completion(&app, &finished_job.title);
            }

            // Leftovers of earlier canceled jobs go away once nothing in this folder can resume
            self.clean_temp_if_idle(&finished_job.spec.output_root, None);
        } else if finished_job.state == JobState::Canceled {
            self.clean_temp_if_idle(&finished_job.spec.output_root, None);
        } else if finished_job.state == JobState::Failed {
            if let (Some(app), Some(err)) = (
                self.app_handle.read().clone(),
                finished_job.error.as_ref(),
            ) {
                if !self.gamemode.status().active {
                    let _ = app
                        .notification()
                        .builder()
                        .title(tr!("Falha no download", "Download failed", "Error en la descarga"))
                        .body(format!("{}: {}", finished_job.title, err.message_pt))
                        .show();
                }
            }
        }

        self.emit_queue_changed();
        self.update_taskbar_progress();
        self.wake_scheduler();
    }

    fn sync_live_progress(&self, live: &Job) {
        {
            let mut list = self.jobs.write();
            if let Some(slot) = list.iter_mut().find(|j| j.id == live.id) {
                slot.state = live.state;
                slot.progress = live.progress.clone();
                slot.log_tail = live.log_tail.clone();
                slot.equivalent_cmd = live.equivalent_cmd.clone();
            }
        }
        if let Some(app) = self.app_handle.read().as_ref() {
            #[derive(Serialize, Clone)]
            struct JobProgressEvent<'a> {
                id: Uuid,
                state: JobState,
                progress: &'a Progress,
                equivalent_cmd: &'a str,
            }
            let _ = app.emit(
                "job:progress",
                JobProgressEvent {
                    id: live.id,
                    state: live.state,
                    progress: &live.progress,
                    equivalent_cmd: &live.equivalent_cmd,
                },
            );
        }
        self.update_taskbar_progress();
    }

    fn update_taskbar_progress(&self) {
        let Some(app) = self.app_handle.read().clone() else {
            return;
        };
        let Some(win) = app.get_webview_window("main") else {
            return;
        };

        let list = self.jobs.read();
        let active: Vec<&Job> = list.iter().filter(|j| j.state.is_active()).collect();
        if !active.is_empty() {
            let avg_pct = active.iter().map(|j| j.progress.pct).sum::<f64>() / (active.len() as f64);
            let _ = win.set_progress_bar(ProgressBarState {
                status: Some(ProgressBarStatus::Normal),
                progress: Some((avg_pct * 100.0).round() as u64),
            });
        } else if list.iter().any(|j| j.state == JobState::Paused) {
            let _ = win.set_progress_bar(ProgressBarState {
                status: Some(ProgressBarStatus::Paused),
                progress: Some(50),
            });
        } else {
            let _ = win.set_progress_bar(ProgressBarState {
                status: Some(ProgressBarStatus::None),
                progress: None,
            });
        }
    }

    fn notify_completion(&self, app: &AppHandle, title: &str) {
        if self.gamemode.status().active {
            return;
        }
        let mut g = self.completed_since_notify.lock();
        if g.1.elapsed() < Duration::from_secs(6) {
            g.0 += 1;
            if g.0 >= 3 && !self.has_active_jobs() {
                let count = g.0;
                g.0 = 0;
                g.1 = Instant::now();
                let _ = app
                    .notification()
                    .builder()
                    .title(tr!("Downloads concluídos", "Downloads finished", "Descargas completadas"))
                    .body(tr!("{count} downloads concluídos.", "{count} downloads finished.", "{count} descargas completadas."))
                    .show();
            }
        } else {
            g.0 = 1;
            g.1 = Instant::now();
            if !self.has_active_jobs() {
                let _ = app
                    .notification()
                    .builder()
                    .title(tr!("Download concluído", "Download finished", "Descarga completada"))
                    .body(title)
                    .show();
            }
        }
    }

    fn mutate_job<F: FnOnce(&mut Job)>(&self, id: Uuid, f: F) {
        let mut updated_clone = None;
        {
            let mut list = self.jobs.write();
            if let Some(j) = list.iter_mut().find(|x| x.id == id) {
                f(j);
                updated_clone = Some(j.clone());
            }
        }
        if let Some(j) = updated_clone {
            let conn = self.db.lock();
            let _ = persist_job_row(&conn, &j);
            drop(conn);
            self.emit_queue_changed();
            self.update_taskbar_progress();
        }
    }

    fn replace_and_persist(&self, updated: &Job) {
        {
            let mut list = self.jobs.write();
            if let Some(slot) = list.iter_mut().find(|x| x.id == updated.id) {
                *slot = updated.clone();
            }
        }
        let conn = self.db.lock();
        let _ = persist_job_row(&conn, updated);
    }

    fn emit_queue_changed(&self) {
        if let Some(app) = self.app_handle.read().as_ref() {
            let snap = self.snapshot();
            let _ = app.emit("queue:changed", snap);
        }
    }

    fn max_priority(&self) -> i32 {
        self.jobs
            .read()
            .iter()
            .map(|j| j.priority)
            .max()
            .unwrap_or(0)
    }

    fn min_priority(&self) -> i32 {
        self.jobs
            .read()
            .iter()
            .map(|j| j.priority)
            .min()
            .unwrap_or(0)
    }
}

fn normalize_host_bucket(platform_id: &str) -> String {
    if platform_id == "youtube_music" {
        "youtube".to_string()
    } else {
        platform_id.to_string()
    }
}

fn persist_job_row(conn: &Connection, job: &Job) -> Result<(), String> {
    let json = serde_json::to_string(job).map_err(|e| e.to_string())?;
    let state_str = serde_json::to_string(&job.state).unwrap_or_else(|_| "\"queued\"".to_string());
    conn.execute(
        "INSERT OR REPLACE INTO queue_jobs (id, priority, created_at_ts, state, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            job.id.to_string(),
            job.priority,
            job.created_at.timestamp_millis(),
            state_str.trim_matches('"'),
            json
        ],
    )
    .map_err(|e| tr!("Erro persistindo fila: {e}", "Error saving the queue: {e}", "Error al guardar la cola: {e}"))?;
    Ok(())
}

fn load_jobs_from_db(conn: &Connection) -> Vec<Job> {
    let Ok(mut stmt) = conn
        .prepare("SELECT payload_json FROM queue_jobs ORDER BY priority DESC, created_at_ts ASC")
    else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|raw| serde_json::from_str::<Job>(&raw).ok())
        .collect()
}

fn extract_media_id_hint(url: &str) -> Option<String> {
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some((_, v)) = parsed.query_pairs().find(|(k, _)| k == "v") {
            let s = v.trim().to_string();
            if s.len() >= 5 {
                return Some(s);
            }
        }
        if let Some(seg) = parsed.path_segments().and_then(|it| it.filter(|s| !s.is_empty()).last())
        {
            if seg.len() >= 5 {
                return Some(seg.to_string());
            }
        }
    }
    None
}

/// Deletes temporary files whose name carries `[<media id>]` (as the output templates write it),
/// searching subfolders up to `depth` levels because yt-dlp mirrors the template folders inside `.boladao-tmp`.
/// Matching the bracketed id keeps generic URL segments from deleting other jobs' files.
fn remove_temp_files_with_id(dir: &Path, media_id: &str, depth: usize) {
    let tag = format!("[{media_id}]");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                remove_temp_files_with_id(&p, media_id, depth - 1);
            }
        } else if p
            .file_name()
            .map(|n| n.to_string_lossy().contains(&tag))
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&p);
        }
    }
}

/// Destination folders whose `.boladao-tmp` can be wiped: every known folder without a job that
/// may still resume from its partial files.
fn idle_temp_roots<'a>(
    jobs: impl IntoIterator<Item = (&'a Path, JobState)>,
    extra_roots: impl IntoIterator<Item = &'a Path>,
) -> Vec<PathBuf> {
    let mut all: BTreeSet<&Path> = BTreeSet::new();
    let mut busy: BTreeSet<&Path> = BTreeSet::new();
    for (root, state) in jobs {
        all.insert(root);
        if state.keeps_partial_files() {
            busy.insert(root);
        }
    }
    all.extend(extra_roots);
    all.into_iter()
        .filter(|root| !busy.contains(root))
        .map(Path::to_path_buf)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_temp_files_of_jobs_that_can_still_resume() {
        let music = Path::new(r"C:\Musicas");
        let video = Path::new(r"C:\Videos");
        let custom = Path::new(r"D:\Custom");
        let jobs = [
            (video, JobState::Paused),
            (video, JobState::Done),
            (custom, JobState::Done),
            (custom, JobState::Canceled),
        ];
        let idle = idle_temp_roots(jobs, [music, video]);
        assert_eq!(idle, vec![music.to_path_buf(), custom.to_path_buf()]);

        for pending in [JobState::Queued, JobState::Downloading, JobState::Failed, JobState::Workshop] {
            assert!(idle_temp_roots([(music, pending)], [music]).is_empty(), "{pending:?} must keep its .part files");
        }
    }

    #[test]
    fn removes_only_temp_files_tagged_with_the_canceled_media_id() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("YouTube").join("duzz");
        fs::create_dir_all(&nested).unwrap();
        let mine = nested.join("Atravessei [WbnEjpLxj70].mp4.part");
        let other = nested.join("Outro video [AAAAAAAAAAA].mp4.part");
        let untagged = nested.join("WbnEjpLxj70 sem colchetes.part");
        for f in [&mine, &other, &untagged] {
            fs::write(f, b"x").unwrap();
        }
        remove_temp_files_with_id(tmp.path(), "WbnEjpLxj70", 6);
        assert!(!mine.exists());
        assert!(other.exists());
        assert!(untagged.exists());
    }
}
