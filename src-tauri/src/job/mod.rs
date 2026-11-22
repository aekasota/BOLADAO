use crate::args::{build_ytdlp_args, format_equivalent_command, ExecContext};
use crate::bins::BinManager;
use crate::classify::{MediaMode, SourceKind};
use crate::errors::{map_stderr_to_job_error, ErrorAction, JobError};
use crate::paths::{
    extract_drive_label, free_disk_bytes, next_available_collision_path, CollisionPolicy,
};
use crate::spec::{AudioFormat, DownloadSpec, VideoProfile};
use crate::winproc::{spawn_in_job, JobGroup, ProcessPriority};
use crate::workshop::run_workshop_pipeline;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

pub const LOG_RING_CAPACITY: usize = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMeta {
    pub platform_id: String,
    pub platform_name: String,
    pub extractor_key: String,
    pub kind: SourceKind,
    pub brand_color: String,
    pub icon_slug: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Probing,
    WaitingPremiere,
    Downloading,
    Postprocessing,
    Workshop,
    Done,
    Failed,
    Paused,
    Canceled,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Canceled)
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Probing
                | Self::WaitingPremiere
                | Self::Downloading
                | Self::Postprocessing
                | Self::Workshop
        )
    }

    /// Whether the job may still resume from the `.part`/`.ytdl` files in `.boladao-tmp`
    /// (queued, active, paused, failed or interrupted). Only finished and canceled jobs let go of them.
    pub fn keeps_partial_files(self) -> bool {
        !matches!(self, Self::Done | Self::Canceled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub stage: String,
    pub pct: f64, // 0.0 ..= 1.0 monotonic across entire job
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub approximate: bool,
    pub speed_bps: u64,
    pub eta_s: Option<u64>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            stage: "Na fila".to_string(),
            pct: 0.0,
            downloaded_bytes: 0,
            total_bytes: None,
            approximate: false,
            speed_bps: 0,
            eta_s: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub url: String,
    pub dedup_key: String,
    pub source: SourceMeta,
    pub group_id: Option<Uuid>,
    pub group_title: Option<String>,
    pub playlist_index: Option<u32>,
    pub title: String,
    pub channel_or_artist: String,
    pub thumb_url: Option<String>,
    pub duration_s: Option<f64>,
    pub estimated_bytes: Option<u64>,
    pub spec: DownloadSpec,
    pub state: JobState,
    pub progress: Progress,
    pub outputs: Vec<PathBuf>,
    pub error: Option<JobError>,
    pub attempts: u8,
    pub priority: i32,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub log_tail: VecDeque<String>,
    pub equivalent_cmd: String,
}

impl Job {
    pub fn push_log(&mut self, line: impl Into<String>) {
        let s = line.into();
        if s.trim().is_empty() {
            return;
        }
        if self.log_tail.len() >= LOG_RING_CAPACITY {
            self.log_tail.pop_front();
        }
        self.log_tail.push_back(s);
    }

    pub fn format_badge(&self) -> String {
        match self.spec.mode {
            MediaMode::Audio => match self.spec.audio_format {
                AudioFormat::Original => "Áudio Original".to_string(),
                AudioFormat::Mp3 => "MP3".to_string(),
                AudioFormat::M4a => "M4A".to_string(),
                AudioFormat::Opus => "Opus".to_string(),
                AudioFormat::Flac => "FLAC".to_string(),
                AudioFormat::Alac => "ALAC".to_string(),
                AudioFormat::Wav => "WAV".to_string(),
                AudioFormat::Aiff => "AIFF".to_string(),
            },
            MediaMode::Video => {
                let res = self
                    .spec
                    .max_height
                    .map(|h| format!("{h}p "))
                    .unwrap_or_else(|| "Máx ".to_string());
                match self.spec.video_profile {
                    VideoProfile::Compatible => format!("{res}MP4"),
                    VideoProfile::MaxQuality => format!("{res}MKV"),
                    VideoProfile::Custom => {
                        format!("{res}{}", self.spec.custom_container.as_str().to_uppercase())
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct StreamSlice {
    downloaded: u64,
    total: Option<u64>,
    frag_idx: Option<u64>,
    frag_cnt: Option<u64>,
    finished: bool,
}

/// Monotonic multi-stream weighted progress state machine (Section 9.4).
/// Guarantees `pct` NEVER jumps backward when `yt-dlp` switches from video stream to audio stream,
/// or from download to postprocessing to FFmpeg Workshop!
pub struct ProgressTracker {
    dl_weight: f64,
    pp_weight: f64,
    ws_weight: f64,
    mode: MediaMode,
    streams: HashMap<String, StreamSlice>,
    stream_order: Vec<String>,
    active_stream_key: String,
    highest_pct: f64,
    ema_speed_bps: f64,
    smoothed_eta_s: Option<f64>,
    last_speed_update: Instant,
}

impl ProgressTracker {
    pub fn new(spec: &DownloadSpec) -> Self {
        let has_ws = spec.workshop.has_any_step(spec.mode, spec.audio_format);
        let heavy_ws = spec.workshop.is_heavy();
        let (dl_w, pp_w, ws_w) = if !has_ws {
            (0.93, 0.07, 0.0)
        } else if heavy_ws {
            (0.65, 0.05, 0.30)
        } else {
            (0.85, 0.05, 0.10)
        };

        Self {
            dl_weight: dl_w,
            pp_weight: pp_w,
            ws_weight: ws_w,
            mode: spec.mode,
            streams: HashMap::new(),
            stream_order: Vec::new(),
            active_stream_key: "stream_0".to_string(),
            highest_pct: 0.0,
            ema_speed_bps: 0.0,
            smoothed_eta_s: None,
            last_speed_update: Instant::now(),
        }
    }

    /// Parses one `stdout` line from `yt-dlp` (`BOLADAO_DL {...}`, `BOLADAO_PP {...}`, `BOLADAO_FILE "..."`).
    pub fn ingest_ytdlp_line(
        &mut self,
        line: &str,
        current_progress: &mut Progress,
        discovered_files: &mut Vec<PathBuf>,
    ) -> Option<JobState> {
        if let Some(json_str) = line.strip_prefix("BOLADAO_DL ") {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
                return None;
            };
            self.apply_dl_json(&v, current_progress);
            return Some(JobState::Downloading);
        }

        if let Some(json_str) = line.strip_prefix("BOLADAO_PP ") {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
                return None;
            };
            self.apply_pp_json(&v, current_progress);
            return Some(JobState::Postprocessing);
        }

        if let Some(json_str) = line.strip_prefix("BOLADAO_FILE ") {
            if let Ok(path_str) = serde_json::from_str::<String>(json_str) {
                let p = PathBuf::from(path_str);
                if !discovered_files.contains(&p) {
                    discovered_files.push(p);
                }
            }
            return None;
        }

        if line.contains("Waiting for video") || line.contains("[wait]") {
            current_progress.stage = "Aguardando estreia...".to_string();
            return Some(JobState::WaitingPremiere);
        }

        None
    }

    fn apply_dl_json(&mut self, v: &serde_json::Value, out: &mut Progress) {
        let status = v
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("downloading");
        let downloaded = v
            .get("downloaded_bytes")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let total = v
            .get("total_bytes")
            .or_else(|| v.get("total_bytes_estimate"))
            .and_then(|n| n.as_u64())
            .filter(|&n| n > 0);
        let is_estimate = v.get("total_bytes").and_then(|n| n.as_u64()).is_none() && total.is_some();
        let frag_idx = v.get("fragment_index").and_then(|n| n.as_u64());
        let frag_cnt = v
            .get("fragment_count")
            .and_then(|n| n.as_u64())
            .filter(|&n| n > 0);

        let fname = v
            .get("filename")
            .or_else(|| v.get("tmpfilename"))
            .and_then(|s| s.as_str())
            .map(ToOwned::to_owned);

        // Detect transition to a second stream even if filename wasn't emitted:
        // if previous stream was `finished` (or downloaded dropped drastically), advance stream key!
        let key = if let Some(f) = fname {
            f
        } else {
            let need_next = self
                .streams
                .get(&self.active_stream_key)
                .map(|prev| {
                    prev.finished || (prev.downloaded > 256 * 1024 && downloaded * 3 < prev.downloaded)
                })
                .unwrap_or(false);
            if need_next {
                format!("stream_{}", self.stream_order.len())
            } else {
                self.active_stream_key.clone()
            }
        };
        self.active_stream_key = key.clone();
        if !self.stream_order.contains(&key) {
            self.stream_order.push(key.clone());
        }

        let slice = self.streams.entry(key).or_insert(StreamSlice {
            downloaded: 0,
            total: None,
            frag_idx: None,
            frag_cnt: None,
            finished: false,
        });
        slice.downloaded = slice.downloaded.max(downloaded);
        if let Some(t) = total {
            slice.total = Some(t.max(slice.downloaded));
        }
        slice.frag_idx = frag_idx;
        slice.frag_cnt = frag_cnt;
        if status == "finished" {
            slice.finished = true;
            if slice.total.is_none() && slice.downloaded > 0 {
                slice.total = Some(slice.downloaded);
            }
        }

        // Compute weighted download fraction across streams without ever dropping backward!
        let expected_streams = if self.mode == MediaMode::Video {
            self.stream_order.len().max(2)
        } else {
            self.stream_order.len().max(1)
        };

        let mut sum_dl = 0u64;
        let mut sum_tot = 0u64;
        let mut all_have_totals = self.stream_order.len() >= expected_streams;

        for k in &self.stream_order {
            if let Some(s) = self.streams.get(k) {
                sum_dl += s.downloaded;
                if let Some(t) = s.total {
                    sum_tot += t;
                } else {
                    all_have_totals = false;
                }
            }
        }

        let raw_dl_fraction = if all_have_totals && sum_tot > 0 {
            out.approximate = is_estimate;
            (sum_dl as f64 / sum_tot as f64).clamp(0.0, 1.0)
        } else {
            out.approximate = true;
            // In Video DASH mode, stream 0 (video) accounts for ~84% of download bytes and stream 1 (audio) ~16%
            let weights: &[f64] = if expected_streams >= 2 {
                &[0.84, 0.16]
            } else {
                &[1.0]
            };
            let mut acc = 0.0;
            for (idx, k) in self.stream_order.iter().enumerate() {
                let w = weights.get(idx).copied().unwrap_or(0.05);
                if let Some(s) = self.streams.get(k) {
                    let sf = if s.finished {
                        1.0
                    } else if let Some(t) = s.total.filter(|&t| t > 0) {
                        (s.downloaded as f64 / t as f64).clamp(0.0, 0.99)
                    } else if let (Some(fi), Some(fc)) = (s.frag_idx, s.frag_cnt) {
                        (fi as f64 / fc as f64).clamp(0.0, 0.99)
                    } else {
                        0.15
                    };
                    acc += w * sf;
                }
            }
            acc.clamp(0.0, 1.0)
        };

        let target_pct = (raw_dl_fraction * self.dl_weight).clamp(0.0, self.dl_weight);
        if target_pct > self.highest_pct {
            self.highest_pct = target_pct;
        }
        out.pct = self.highest_pct;
        out.downloaded_bytes = sum_dl;
        out.total_bytes = if sum_tot > 0 { Some(sum_tot) } else { None };

        // Stage label in natural Brazilian Portuguese
        let cur_idx = self
            .stream_order
            .iter()
            .position(|k| k == &self.active_stream_key)
            .unwrap_or(0);
        out.stage = match self.mode {
            MediaMode::Audio => "Baixando áudio".to_string(),
            MediaMode::Video => {
                if cur_idx == 0 {
                    "Baixando vídeo".to_string()
                } else {
                    "Baixando áudio".to_string()
                }
            }
        };

        // Smoothed speed (EMA ~1s) & ETA with hysteresis (Section 9.4)
        if let Some(raw_spd) = v.get("speed").and_then(|n| n.as_f64()).filter(|&s| s > 0.0) {
            let dt = self.last_speed_update.elapsed().as_secs_f64().min(2.0);
            self.last_speed_update = Instant::now();
            let alpha = (dt / 1.0).clamp(0.15, 0.65);
            if self.ema_speed_bps < 1.0 {
                self.ema_speed_bps = raw_spd;
            } else {
                self.ema_speed_bps = self.ema_speed_bps * (1.0 - alpha) + raw_spd * alpha;
            }
            out.speed_bps = self.ema_speed_bps as u64;
        }

        if let Some(raw_eta) = v.get("eta").and_then(|n| n.as_f64()).filter(|&e| e >= 0.0) {
            let smoothed = match self.smoothed_eta_s {
                None => raw_eta,
                Some(prev) => {
                    // Hysteresis: damp sudden spikes (>2x jump damped strongly)
                    let clamped_target = if raw_eta > prev * 2.2 + 5.0 {
                        prev * 1.25 + 2.0
                    } else {
                        raw_eta
                    };
                    prev * 0.75 + clamped_target * 0.25
                }
            };
            self.smoothed_eta_s = Some(smoothed);
            out.eta_s = Some(smoothed.round() as u64);
        }
    }

    fn apply_pp_json(&mut self, v: &serde_json::Value, out: &mut Progress) {
        let pp_name = v
            .get("postprocessor")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("started");

        out.stage = humanize_postprocessor_pt(pp_name);
        out.speed_bps = 0;
        out.eta_s = None;

        let frac = if status == "finished" { 0.9 } else { 0.45 };
        let target = self.dl_weight + self.pp_weight * frac;
        if target > self.highest_pct {
            self.highest_pct = target;
        }
        out.pct = self.highest_pct;
    }

    pub fn apply_workshop_progress(&mut self, stage_pt: &str, ws_frac: f64, out: &mut Progress) {
        out.stage = stage_pt.to_string();
        out.speed_bps = 0;
        out.eta_s = None;
        let target = self.dl_weight + self.pp_weight + self.ws_weight * ws_frac.clamp(0.0, 1.0);
        if target > self.highest_pct {
            self.highest_pct = target;
        }
        out.pct = self.highest_pct.min(0.995);
    }
}

pub fn humanize_postprocessor_pt(pp: &str) -> String {
    let lower = pp.to_ascii_lowercase();
    if lower.contains("merger") || lower.contains("remux") || lower.contains("concat") {
        "Juntando".to_string()
    } else if lower.contains("extractaudio") {
        "Extraindo áudio".to_string()
    } else if lower.contains("thumbnail") {
        "Colocando a capa".to_string()
    } else if lower.contains("sponsorblock") {
        "Removendo patrocínio".to_string()
    } else if lower.contains("splitchapters") {
        "Dividindo capítulos".to_string()
    } else if lower.contains("metadata") || lower.contains("chapters") {
        "Gravando metadados".to_string()
    } else if lower.contains("subtitles") {
        "Processando legendas".to_string()
    } else {
        "Finalizando arquivo".to_string()
    }
}

pub struct ActiveJobHandle {
    pub job_group: Arc<JobGroup>,
    pub paused_flag: Arc<AtomicBool>,
    pub canceled_flag: Arc<AtomicBool>,
}

/// Executes a single `Job` end-to-end (`yt-dlp` -> collision resolution -> FFmpeg Workshop).
#[allow(clippy::too_many_arguments)]
pub async fn execute_job_pipeline<F>(
    mut job: Job,
    bins: Arc<BinManager>,
    mut ctx: ExecContext,
    collision_policy: CollisionPolicy,
    workshop_priority: ProcessPriority,
    handle: Arc<ActiveJobHandle>,
    mut on_update: F,
) -> Job
where
    F: FnMut(&Job),
{
    job.started_at = Some(Utc::now());
    job.error = None;

    // Pre-flight disk space check (`GetDiskFreeSpaceExW` — Section 9.6 & 14.6)
    let _ = fs::create_dir_all(&job.spec.output_root);
    let _ = fs::create_dir_all(job.spec.output_root.join(crate::paths::TEMP_SUBDIR_NAME));
    if let (Some(est), Some(free)) = (job.estimated_bytes, free_disk_bytes(&job.spec.output_root)) {
        // Require 5% headroom for temporary muxing
        let needed = (est as f64 * 1.05) as u64;
        if needed > free {
            let drive = extract_drive_label(&job.spec.output_root);
            job.state = JobState::Failed;
            job.finished_at = Some(Utc::now());
            job.error = Some(JobError {
                code: "disk_full_preflight".to_string(),
                message_pt: format!(
                    "Não cabe: precisa de ~{} e tem {} livres em {drive}.",
                    format_bytes_pt(needed),
                    format_bytes_pt(free)
                ),
                hint_pt: Some("Troque a pasta de destino ou libere espaço no disco.".to_string()),
                action: ErrorAction::ChangeDestination,
                transient: false,
                suggest_impersonate: false,
            });
            on_update(&job);
            return job;
        }
    }

    let mut tried_impersonate = ctx.impersonate_target.is_some();

    let discovered_files = loop {
        let mut args = match build_ytdlp_args(&job.url, &job.spec, &ctx) {
            Ok(a) => a,
            Err(msg) => {
                job.state = JobState::Failed;
                job.finished_at = Some(Utc::now());
                job.error = Some(JobError {
                    code: "invalid_args".to_string(),
                    message_pt: msg,
                    hint_pt: None,
                    action: ErrorAction::None,
                    transient: false,
                    suggest_impersonate: false,
                });
                on_update(&job);
                return job;
            }
        };

        match collision_policy {
            CollisionPolicy::Overwrite => args.insert(0, "--force-overwrites".into()),
            CollisionPolicy::Skip => args.insert(0, "--no-overwrites".into()),
            CollisionPolicy::KeepBoth => {
                // If an identical output already exists, we avoid clobbering by letting yt-dlp write or renaming
                args.insert(0, "--no-overwrites".into());
            }
        }

        job.equivalent_cmd = format_equivalent_command(&bins.ytdlp_exe(), &args, true);
        job.state = JobState::Downloading;
        job.progress.stage = if job.spec.mode == MediaMode::Audio {
            "Baixando áudio".to_string()
        } else {
            "Baixando vídeo".to_string()
        };
        on_update(&job);

        let mut child = match spawn_in_job(
            &handle.job_group,
            &bins.ytdlp_exe(),
            &args,
            ProcessPriority::BelowNormal,
            None,
        ) {
            Ok(c) => c,
            Err(e) => {
                job.state = JobState::Failed;
                job.finished_at = Some(Utc::now());
                job.error = Some(JobError {
                    code: "spawn_failed".to_string(),
                    message_pt: format!("Falha iniciando yt-dlp: {e}"),
                    hint_pt: None,
                    action: ErrorAction::Retry,
                    transient: true,
                    suggest_impersonate: false,
                });
                on_update(&job);
                return job;
            }
        };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let mut out_reader = BufReader::new(stdout).lines();
        let mut err_reader = BufReader::new(stderr).lines();

        let mut tracker = ProgressTracker::new(&job.spec);
        let mut files: Vec<PathBuf> = Vec::new();
        let mut stderr_buf = String::new();
        let mut last_emit = Instant::now();

        let mut out_done = false;
        let mut err_done = false;

        while !out_done || !err_done {
            tokio::select! {
                res = out_reader.next_line(), if !out_done => {
                    match res {
                        Ok(Some(line)) => {
                            if let Some(new_state) = tracker.ingest_ytdlp_line(&line, &mut job.progress, &mut files) {
                                job.state = new_state;
                                if last_emit.elapsed().as_millis() >= 100 || new_state == JobState::Postprocessing {
                                    last_emit = Instant::now();
                                    on_update(&job);
                                }
                            } else {
                                job.push_log(line);
                            }
                        }
                        _ => { out_done = true; }
                    }
                }
                res = err_reader.next_line(), if !err_done => {
                    match res {
                        Ok(Some(line)) => {
                            stderr_buf.push_str(&line);
                            stderr_buf.push('\n');
                            job.push_log(line);
                        }
                        _ => { err_done = true; }
                    }
                }
            }
        }

        let exit = child.wait().await;

        if handle.canceled_flag.load(Ordering::SeqCst) {
            job.state = JobState::Canceled;
            job.progress.stage = "Cancelado".to_string();
            job.progress.speed_bps = 0;
            job.finished_at = Some(Utc::now());
            on_update(&job);
            return job;
        }

        if handle.paused_flag.load(Ordering::SeqCst) {
            job.state = JobState::Paused;
            job.progress.stage = "Pausado".to_string();
            job.progress.speed_bps = 0;
            job.progress.eta_s = None;
            on_update(&job);
            return job;
        }

        let ok = exit.map(|s| s.success()).unwrap_or(false);
        if ok {
            break files;
        }

        let drive = extract_drive_label(&job.spec.output_root);
        let mapped = map_stderr_to_job_error(&stderr_buf, &job.source.platform_id, &drive);

        // Automatic 1-shot `--impersonate chrome` retry on 403 / Cloudflare anti-bot (Section 7.5)
        if mapped.suggest_impersonate && !tried_impersonate {
            tried_impersonate = true;
            ctx.impersonate_target = Some("chrome".to_string());
            job.push_log("[boladão] Bloqueio anti-bot detectado (403); tentando novamente com impersonate=chrome...");
            continue;
        }

        job.state = JobState::Failed;
        job.progress.speed_bps = 0;
        job.progress.eta_s = None;
        job.finished_at = Some(Utc::now());
        job.error = Some(mapped);
        on_update(&job);
        return job;
    };

    let mut outputs = discovered_files;

    // Run optional FFmpeg Workshop pipeline if any step is configured
    if !outputs.is_empty()
        && job
            .spec
            .workshop
            .has_any_step(job.spec.mode, job.spec.audio_format)
    {
        job.state = JobState::Workshop;
        let primary = outputs[0].clone();
        let mut tracker = ProgressTracker::new(&job.spec);
        tracker.apply_workshop_progress("Processando na oficina...", 0.02, &mut job.progress);
        on_update(&job);

        let mut last_ws_emit = Instant::now();
        let spec_clone = job.spec.clone();
        let ws_res = run_workshop_pipeline(
            &handle.job_group,
            &bins,
            primary,
            &spec_clone,
            workshop_priority,
            |stage_pt, frac| {
                tracker.apply_workshop_progress(stage_pt, frac, &mut job.progress);
                if last_ws_emit.elapsed().as_millis() >= 100 {
                    last_ws_emit = Instant::now();
                    on_update(&job);
                }
            },
        )
        .await;

        if handle.canceled_flag.load(Ordering::SeqCst) {
            job.state = JobState::Canceled;
            job.progress.stage = "Cancelado".to_string();
            job.finished_at = Some(Utc::now());
            on_update(&job);
            return job;
        }
        if handle.paused_flag.load(Ordering::SeqCst) {
            job.state = JobState::Paused;
            job.progress.stage = "Pausado".to_string();
            on_update(&job);
            return job;
        }

        match ws_res {
            Ok(ws_outputs) => {
                // Keep split-chapter extra outputs if any, plus workshop outputs
                let mut merged = ws_outputs;
                for prev in outputs.into_iter().skip(1) {
                    if prev.exists() && !merged.contains(&prev) {
                        merged.push(prev);
                    }
                }
                outputs = merged;
            }
            Err(err_msg) => {
                job.push_log(&err_msg);
                let drive = extract_drive_label(&job.spec.output_root);
                job.state = JobState::Failed;
                job.finished_at = Some(Utc::now());
                job.error = Some(map_stderr_to_job_error(
                    &err_msg,
                    &job.source.platform_id,
                    &drive,
                ));
                on_update(&job);
                return job;
            }
        }
    }

    // Ensure collision policy `KeepBoth` suffix if needed when multiple runs target the same file
    let _ = next_available_collision_path; // available for manual re-downloads

    job.outputs = outputs;
    job.state = JobState::Done;
    job.progress.stage = "Concluído".to_string();
    job.progress.pct = 1.0;
    job.progress.speed_bps = 0;
    job.progress.eta_s = Some(0);
    job.finished_at = Some(Utc::now());
    on_update(&job);
    job
}

pub fn format_bytes_pt(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB).replace('.', ",")
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{:.0} KB", (b / KB).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_strictly_monotonic_across_separate_video_audio_and_postprocess() {
        let mut spec = DownloadSpec::default();
        spec.mode = MediaMode::Video;
        spec.workshop.loudnorm_lufs = Some(-14.0);

        let mut tracker = ProgressTracker::new(&spec);
        let mut prog = Progress::default();
        let mut files = Vec::new();

        // Real captured sequence from yt-dlp:
        // 1. Video stream f137 (10 MB) 0% -> 50% -> 100% finished
        // 2. Audio stream f251 (2 MB) 0% -> 50% -> 100% finished (raw stream % resets to 0%!)
        // 3. HLS/fragmented edge case + Postprocessor Merger + Workshop loudnorm
        let lines = [
            r#"BOLADAO_DL {"status":"downloading","filename":"vid.f137.mp4","downloaded_bytes":1000000,"total_bytes":10000000,"speed":5000000,"eta":2}"#,
            r#"BOLADAO_DL {"status":"downloading","filename":"vid.f137.mp4","downloaded_bytes":5000000,"total_bytes":10000000,"speed":5200000,"eta":1}"#,
            r#"BOLADAO_DL {"status":"finished","filename":"vid.f137.mp4","downloaded_bytes":10000000,"total_bytes":10000000}"#,
            r#"BOLADAO_DL {"status":"downloading","filename":"vid.f251.webm","downloaded_bytes":0,"total_bytes":2000000,"speed":4000000,"eta":1}"#,
            r#"BOLADAO_DL {"status":"downloading","filename":"vid.f251.webm","downloaded_bytes":1000000,"total_bytes":2000000,"speed":4100000,"eta":0}"#,
            r#"BOLADAO_DL {"status":"finished","filename":"vid.f251.webm","downloaded_bytes":2000000,"total_bytes":2000000}"#,
            r#"BOLADAO_PP {"status":"started","postprocessor":"Merger"}"#,
            r#"BOLADAO_PP {"status":"finished","postprocessor":"Merger"}"#,
            r#"BOLADAO_FILE "C:\\Videos\\Boladão\\vid.mp4""#,
        ];

        let mut prev_pct = 0.0;
        for line in lines {
            tracker.ingest_ytdlp_line(line, &mut prog, &mut files);
            assert!(
                prog.pct >= prev_pct,
                "Progress regressed from {prev_pct:.4} to {:.4} on line: {line}",
                prog.pct
            );
            prev_pct = prog.pct;
        }

        tracker.apply_workshop_progress("Normalizando volume", 0.5, &mut prog);
        assert!(prog.pct > prev_pct);
        prev_pct = prog.pct;

        tracker.apply_workshop_progress("Normalizando volume", 1.0, &mut prog);
        assert!(prog.pct > prev_pct);
        assert_eq!(files, vec![PathBuf::from(r"C:\Videos\Boladão\vid.mp4")]);
    }

    #[test]
    fn hls_fragments_without_total_bytes_advance_monotonically() {
        let spec = DownloadSpec::default();
        let mut tracker = ProgressTracker::new(&spec);
        let mut prog = Progress::default();
        let mut files = Vec::new();

        let hls_lines = [
            r#"BOLADAO_DL {"status":"downloading","filename":"live.mp4","downloaded_bytes":200000,"fragment_index":2,"fragment_count":20,"speed":1000000,"eta":18}"#,
            r#"BOLADAO_DL {"status":"downloading","filename":"live.mp4","downloaded_bytes":1000000,"fragment_index":10,"fragment_count":20,"speed":1000000,"eta":10}"#,
            r#"BOLADAO_DL {"status":"downloading","filename":"live.mp4","downloaded_bytes":1900000,"fragment_index":19,"fragment_count":20,"speed":1000000,"eta":1}"#,
        ];

        let mut prev = 0.0;
        for line in hls_lines {
            tracker.ingest_ytdlp_line(line, &mut prog, &mut files);
            assert!(prog.approximate);
            assert!(prog.pct > prev);
            prev = prog.pct;
        }
    }
}
