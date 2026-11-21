use crate::bins::BinManager;
use crate::classify::MediaMode;
use crate::paths::{app_config_dir, app_local_dir};
use crate::spec::DownloadSpec;
use crate::winproc::{run_hidden_output, ProcessPriority};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MediaProbeDetails {
    pub container_format: Option<String>,
    pub file_size_bytes: u64,
    pub duration_s: Option<f64>,
    pub video_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub audio_codec: Option<String>,
    pub sample_rate_hz: Option<u32>,
    pub bits_per_sample: Option<u32>,
    pub channels: Option<u8>,
    pub overall_bitrate_kbps: Option<u32>,
    pub cover_dimensions: Option<(u32, u32)>,
    pub integrated_lufs: Option<f32>,
    pub lossy_cutoff_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub id: Uuid,
    pub url: String,
    pub dedup_key: String,
    pub platform_id: String,
    pub platform_name: String,
    pub title: String,
    pub channel_or_artist: String,
    pub thumb_url: Option<String>,
    pub is_playlist_origin: bool,
    pub playlist_url: Option<String>,
    pub mode: MediaMode,
    pub format_badge: String,
    pub primary_path: PathBuf,
    pub extra_paths: Vec<PathBuf>,
    pub file_exists: bool,
    pub waveform_url: Option<String>,
    pub spectrogram_url: Option<String>,
    pub details: MediaProbeDetails,
    pub equivalent_cmd: String,
    pub spec: DownloadSpec,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryQuery {
    pub search: Option<String>,
    pub mode: Option<MediaMode>,
    pub platform_id: Option<String>,
    pub days_back: Option<u32>,
}

pub fn waveforms_cache_dir() -> PathBuf {
    let d = app_local_dir().join("cache").join("waveforms");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn spectrograms_cache_dir() -> PathBuf {
    let d = app_local_dir().join("cache").join("spectrograms");
    let _ = fs::create_dir_all(&d);
    d
}

pub struct LibraryStore {
    conn: Mutex<Connection>,
}

impl LibraryStore {
    pub fn open_default() -> Result<Self, String> {
        let db_path = app_config_dir().join("boladao.db");
        Self::open_at(&db_path)
    }

    pub fn open_at(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path).map_err(|e| format!("Erro abrindo SQLite: {e}"))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS library_items (
                id TEXT PRIMARY KEY,
                created_at_ts INTEGER NOT NULL,
                dedup_key TEXT NOT NULL,
                platform_id TEXT NOT NULL,
                mode TEXT NOT NULL,
                search_blob TEXT NOT NULL,
                payload_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_lib_created ON library_items(created_at_ts DESC);
             CREATE INDEX IF NOT EXISTS idx_lib_dedup ON library_items(dedup_key);",
        )
        .map_err(|e| format!("Erro criando tabela library_items: {e}"))?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert_or_update(&self, entry: &LibraryEntry) -> Result<(), String> {
        let json = serde_json::to_string(entry).map_err(|e| e.to_string())?;
        let mode_str = match entry.mode {
            MediaMode::Audio => "audio",
            MediaMode::Video => "video",
        };
        let search_blob = format!(
            "{} {} {} {}",
            entry.title.to_lowercase(),
            entry.channel_or_artist.to_lowercase(),
            entry.platform_name.to_lowercase(),
            entry.format_badge.to_lowercase()
        );
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO library_items
             (id, created_at_ts, dedup_key, platform_id, mode, search_blob, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id.to_string(),
                entry.created_at.timestamp(),
                entry.dedup_key,
                entry.platform_id,
                mode_str,
                search_blob,
                json
            ],
        )
        .map_err(|e| format!("Erro salvando na Biblioteca: {e}"))?;
        Ok(())
    }

    pub fn has_downloaded_before(&self, dedup_key: &str) -> Option<LibraryEntry> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM library_items WHERE dedup_key = ?1 ORDER BY created_at_ts DESC LIMIT 1",
            )
            .ok()?;
        let raw: String = stmt.query_row(params![dedup_key], |r| r.get(0)).ok()?;
        let mut entry: LibraryEntry = serde_json::from_str(&raw).ok()?;
        entry.file_exists = entry.primary_path.is_file();
        Some(entry)
    }

    pub fn get_by_id(&self, id: Uuid) -> Option<LibraryEntry> {
        let conn = self.conn.lock();
        let raw: String = conn
            .query_row(
                "SELECT payload_json FROM library_items WHERE id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .ok()?;
        let mut entry: LibraryEntry = serde_json::from_str(&raw).ok()?;
        entry.file_exists = entry.primary_path.is_file();
        Some(entry)
    }

    pub fn query(&self, q: &LibraryQuery) -> Result<Vec<LibraryEntry>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT payload_json FROM library_items ORDER BY created_at_ts DESC LIMIT 500")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;

        let needle = q
            .search
            .as_ref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        let cutoff_ts = q
            .days_back
            .map(|days| Utc::now().timestamp() - (days as i64) * 86_400);

        let mut out = Vec::new();
        for r in rows.flatten() {
            let Ok(mut item) = serde_json::from_str::<LibraryEntry>(&r) else {
                continue;
            };
            if let Some(m) = q.mode {
                if item.mode != m {
                    continue;
                }
            }
            if let Some(pid) = &q.platform_id {
                if !pid.is_empty() && pid != "all" && &item.platform_id != pid {
                    continue;
                }
            }
            if let Some(min_ts) = cutoff_ts {
                if item.created_at.timestamp() < min_ts {
                    continue;
                }
            }
            if let Some(s) = &needle {
                let hay = format!(
                    "{} {} {} {}",
                    item.title.to_lowercase(),
                    item.channel_or_artist.to_lowercase(),
                    item.platform_name.to_lowercase(),
                    item.primary_path.to_string_lossy().to_lowercase()
                );
                if !hay.contains(s) {
                    continue;
                }
            }
            item.file_exists = item.primary_path.is_file();
            out.push(item);
        }
        Ok(out)
    }

    pub fn delete_entry(&self, id: Uuid, delete_disk_file: bool) -> Result<(), String> {
        if let Some(entry) = self.get_by_id(id) {
            if delete_disk_file {
                if entry.primary_path.is_file() {
                    fs::remove_file(&entry.primary_path)
                        .map_err(|e| format!("Não foi possível apagar o arquivo: {e}"))?;
                }
                for extra in &entry.extra_paths {
                    let _ = fs::remove_file(extra);
                }
            }
            let _ = fs::remove_file(waveforms_cache_dir().join(format!("{id}.png")));
            let _ = fs::remove_file(spectrograms_cache_dir().join(format!("{id}.png")));
        }
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM library_items WHERE id = ?1",
            params![id.to_string()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Runs `ffprobe` + loudness scan + waveform PNG generation for newly completed media (Section 8.4).
pub async fn analyze_completed_file(
    id: Uuid,
    path: &Path,
    mode: MediaMode,
    bins: &BinManager,
) -> (MediaProbeDetails, Option<String>) {
    let mut details = MediaProbeDetails::default();
    if let Ok(meta) = fs::metadata(path) {
        details.file_size_bytes = meta.len();
    }

    let ffprobe_args = vec![
        OsString::from("-v"),
        OsString::from("quiet"),
        OsString::from("-print_format"),
        OsString::from("json"),
        OsString::from("-show_format"),
        OsString::from("-show_streams"),
        path.as_os_str().to_os_string(),
    ];

    if let Ok((0, stdout, _)) =
        run_hidden_output(&bins.ffprobe_exe(), &ffprobe_args, ProcessPriority::BelowNormal).await
    {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&stdout) {
            if let Some(fmt) = v.get("format") {
                details.container_format = fmt
                    .get("format_name")
                    .and_then(|s| s.as_str())
                    .map(|s| s.split(',').next().unwrap_or(s).to_string());
                details.duration_s = fmt
                    .get("duration")
                    .and_then(|s| s.as_str())
                    .and_then(|s| s.parse::<f64>().ok());
                details.overall_bitrate_kbps = fmt
                    .get("bit_rate")
                    .and_then(|s| s.as_str())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|b| (b / 1000) as u32);
            }
            if let Some(streams) = v.get("streams").and_then(|s| s.as_array()) {
                for st in streams {
                    let ctype = st.get("codec_type").and_then(|s| s.as_str()).unwrap_or("");
                    let cname = st.get("codec_name").and_then(|s| s.as_str()).unwrap_or("");
                    let attached_pic = st
                        .get("disposition")
                        .and_then(|d| d.get("attached_pic"))
                        .and_then(|n| n.as_i64())
                        .unwrap_or(0)
                        == 1;

                    if ctype == "video" && attached_pic {
                        if let (Some(w), Some(h)) = (
                            st.get("width").and_then(|n| n.as_u64()),
                            st.get("height").and_then(|n| n.as_u64()),
                        ) {
                            details.cover_dimensions = Some((w as u32, h as u32));
                        }
                    } else if ctype == "video" && details.video_codec.is_none() {
                        details.video_codec = Some(cname.to_string());
                        details.width = st.get("width").and_then(|n| n.as_u64()).map(|n| n as u32);
                        details.height = st.get("height").and_then(|n| n.as_u64()).map(|n| n as u32);
                        if let Some(fr) = st.get("avg_frame_rate").and_then(|s| s.as_str()) {
                            if let Some((num, den)) = fr.split_once('/') {
                                if let (Ok(n), Ok(d)) = (num.parse::<f32>(), den.parse::<f32>()) {
                                    if d > 0.0 {
                                        details.fps = Some((n / d * 100.0).round() / 100.0);
                                    }
                                }
                            }
                        }
                    } else if ctype == "audio" && details.audio_codec.is_none() {
                        details.audio_codec = Some(cname.to_string());
                        details.sample_rate_hz = st
                            .get("sample_rate")
                            .and_then(|s| s.as_str())
                            .and_then(|s| s.parse::<u32>().ok());
                        details.bits_per_sample = st
                            .get("bits_per_raw_sample")
                            .and_then(|s| s.as_str())
                            .and_then(|s| s.parse::<u32>().ok())
                            .or_else(|| {
                                st.get("bits_per_sample")
                                    .and_then(|n| n.as_u64())
                                    .filter(|&n| n > 0)
                                    .map(|n| n as u32)
                            });
                        details.channels =
                            st.get("channels").and_then(|n| n.as_u64()).map(|n| n as u8);
                    }
                }
            }
        }
    }

    // Measure integrated loudness (LUFS) on up to the first 180s so large files stay responsive
    if details.audio_codec.is_some() {
        let lufs_args = vec![
            OsString::from("-hide_banner"),
            OsString::from("-nostats"),
            OsString::from("-t"),
            OsString::from("180"),
            OsString::from("-i"),
            path.as_os_str().to_os_string(),
            OsString::from("-vn"),
            OsString::from("-af"),
            OsString::from("loudnorm=print_format=json"),
            OsString::from("-f"),
            OsString::from("null"),
            OsString::from("-"),
        ];
        if let Ok((_, _, stderr)) =
            run_hidden_output(&bins.ffmpeg_exe(), &lufs_args, ProcessPriority::Idle).await
        {
            if let (Some(a), Some(b)) = (stderr.rfind('{'), stderr.rfind('}')) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&stderr[a..=b]) {
                    details.integrated_lufs = val
                        .get("input_i")
                        .and_then(|s| s.as_str())
                        .and_then(|s| s.parse::<f32>().ok())
                        .filter(|v| v.is_finite());
                }
            }
        }
    }

    // Generate transparent 1200x96 waveform PNG for audio entries (`showwavespic`)
    let mut waveform_url = None;
    if mode == MediaMode::Audio && bins.status().capabilities.has_showwavespic {
        let fname = format!("{id}.png");
        let wave_out = waveforms_cache_dir().join(&fname);
        let wave_args = vec![
            OsString::from("-hide_banner"),
            OsString::from("-y"),
            OsString::from("-i"),
            path.as_os_str().to_os_string(),
            OsString::from("-vn"),
            OsString::from("-lavfi"),
            OsString::from("aformat=channel_layouts=mono,showwavespic=s=1200x96:colors=#FA023E"),
            OsString::from("-frames:v"),
            OsString::from("1"),
            wave_out.as_os_str().to_os_string(),
        ];
        if let Ok((0, _, _)) =
            run_hidden_output(&bins.ffmpeg_exe(), &wave_args, ProcessPriority::Idle).await
        {
            if wave_out.is_file() {
                waveform_url = Some(format!("http://boladaomedia.localhost/wave/{fname}"));
            }
        }
    }

    (details, waveform_url)
}

/// Generates an on-demand spectrogram PNG (`showspectrumpic`) AND checks high-frequency energy (>16.5 kHz vs midband)
/// to detect lossy source shelf cuts (Section 8.4: "Agudos cortam em ~16 kHz: a fonte tinha perdas").
pub async fn generate_spectrogram_and_cutoff(
    id: Uuid,
    path: &Path,
    bins: &BinManager,
) -> Result<(String, Option<String>), String> {
    if !path.is_file() {
        return Err("Arquivo não encontrado no disco.".to_string());
    }
    let fname = format!("{id}.png");
    let spec_out = spectrograms_cache_dir().join(&fname);

    if !spec_out.is_file() {
        let args = vec![
            OsString::from("-hide_banner"),
            OsString::from("-y"),
            OsString::from("-t"),
            OsString::from("180"),
            OsString::from("-i"),
            path.as_os_str().to_os_string(),
            OsString::from("-vn"),
            OsString::from("-lavfi"),
            OsString::from("showspectrumpic=s=1024x360:mode=combined:color=fire:legend=1"),
            OsString::from("-frames:v"),
            OsString::from("1"),
            spec_out.as_os_str().to_os_string(),
        ];
        let (code, _, stderr) =
            run_hidden_output(&bins.ffmpeg_exe(), &args, ProcessPriority::BelowNormal)
                .await
                .map_err(|e| format!("Falha gerando espectrograma: {e}"))?;
        if code != 0 || !spec_out.is_file() {
            return Err(format!("FFmpeg não conseguiu gerar espectrograma: {stderr}"));
        }
    }

    // Measure RMS dB in midband (2k-10k Hz) vs ultra-high band (>16.5k Hz) via `astats`
    let cutoff_note = detect_16khz_shelf_cutoff(path, bins).await;
    Ok((
        format!("http://boladaomedia.localhost/spec/{fname}"),
        cutoff_note,
    ))
}

async fn detect_16khz_shelf_cutoff(path: &Path, bins: &BinManager) -> Option<String> {
    let measure_band_rms = |filter: &str| {
        let ff = bins.ffmpeg_exe();
        let p = path.to_path_buf();
        let flt = format!("{filter},astats=metadata=1:reset=0");
        async move {
            let args = vec![
                OsString::from("-hide_banner"),
                OsString::from("-t"),
                OsString::from("45"),
                OsString::from("-i"),
                p.as_os_str().to_os_string(),
                OsString::from("-vn"),
                OsString::from("-af"),
                OsString::from(flt),
                OsString::from("-f"),
                OsString::from("null"),
                OsString::from("-"),
            ];
            let (_, _, stderr) = run_hidden_output(&ff, &args, ProcessPriority::Idle)
                .await
                .ok()?;
            stderr.lines().rev().find_map(|l| {
                if l.contains("RMS level dB:") {
                    let v = l.split("RMS level dB:").nth(1)?.trim();
                    if v == "-inf" {
                        Some(-120.0_f32)
                    } else {
                        v.parse::<f32>().ok()
                    }
                } else {
                    None
                }
            })
        }
    };

    let (mid_rms, high_rms) = tokio::join!(
        measure_band_rms("highpass=f=2000,lowpass=f=10000"),
        measure_band_rms("highpass=f=16800")
    );

    if let (Some(mid), Some(hi)) = (mid_rms, high_rms) {
        // If midband has normal musical energy (> -45 dBFS) and >16.8 kHz drops by > 36 dB (or below -78 dB),
        // there is a clear brickwall lowpass around ~16 kHz typical of 128-160 kbps lossy codecs!
        if mid > -45.0 && (mid - hi > 36.0 || hi < -78.0) {
            return Some("Agudos cortam em ~16 kHz: a fonte tinha perdas.".to_string());
        }
    }
    None
}
