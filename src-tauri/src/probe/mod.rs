use crate::args::CookieSource;
use crate::bins::BinManager;
use crate::classify::{classify_url, refine_with_extractor_key, Classification, SourceKind};
use crate::errors::{map_stderr_to_job_error, JobError};
use crate::paths::app_local_dir;
use crate::winproc::{run_hidden_output, ProcessPriority};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeScope {
    SingleVideo,
    FullPlaylist,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFormatOption {
    pub height: u32,
    pub fps: u32,
    pub label: String,
    pub has_hdr: bool,
    pub has_h264: bool,
    pub vcodecs: Vec<String>,
    pub estimated_total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleTrackInfo {
    pub lang: String,
    pub name: String,
    pub is_auto: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterInfo {
    pub title: String,
    pub start_time: f64,
    pub end_time: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistEntryPreview {
    pub index: u32,
    pub id: String,
    pub url: String,
    pub title: String,
    pub uploader: Option<String>,
    pub duration_s: Option<f64>,
    pub thumb_url: Option<String>,
    pub is_short: bool,
    pub is_live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub url: String,
    pub classification: Classification,
    pub extractor_key: String,
    pub id: String,
    pub title: String,
    pub channel_or_artist: String,
    pub upload_date: Option<String>, // YYYY-MM-DD
    pub duration_s: Option<f64>,
    pub thumb_url: Option<String>,
    pub is_portrait: bool,
    pub is_playlist: bool,
    pub playlist_title: Option<String>,
    pub playlist_count: Option<usize>,
    pub entries: Vec<PlaylistEntryPreview>,
    pub video_resolutions: Vec<VideoFormatOption>,
    pub best_audio_bytes: Option<u64>,
    pub best_audio_acodec: Option<String>,
    pub audio_languages: Vec<String>,
    pub manual_subtitles: Vec<SubtitleTrackInfo>,
    pub auto_subtitles: Vec<SubtitleTrackInfo>,
    pub chapters: Vec<ChapterInfo>,
    pub is_live: bool,
    pub is_upcoming: bool,
    pub release_timestamp: Option<i64>,
}

struct CachedProbe {
    created_at: Instant,
    result: ProbeResult,
}

pub struct ProbeService {
    cache: Mutex<HashMap<(String, ProbeScope), CachedProbe>>,
}

impl ProbeService {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub async fn probe_url(
        &self,
        raw_url: &str,
        scope: ProbeScope,
        bins: &BinManager,
        proxy_url: Option<String>,
        cookies: CookieSource,
    ) -> Result<ProbeResult, JobError> {
        let initial_cls = classify_url(raw_url).map_err(|msg| JobError {
            code: "invalid_url".to_string(),
            message_pt: msg,
            hint_pt: None,
            action: crate::errors::ErrorAction::None,
            transient: false,
            suggest_impersonate: false,
        })?;

        if initial_cls.is_drm {
            return Err(JobError {
                code: "drm".to_string(),
                message_pt: initial_cls
                    .drm_message
                    .unwrap_or_else(|| {
                    tr!(
                        "Esse serviço usa DRM. O yt-dlp não baixa daqui.",
                        "This service uses DRM. yt-dlp can't download from it.",
                        "Este servicio usa DRM. yt-dlp no puede descargar de aquí."
                    )
                }),
                hint_pt: None,
                action: crate::errors::ErrorAction::None,
                transient: false,
                suggest_impersonate: false,
            });
        }

        let cache_key = (initial_cls.normalized_url.clone(), scope);
        {
            let mut g = self.cache.lock();
            g.retain(|_, v| v.created_at.elapsed() < Duration::from_secs(600));
            if let Some(hit) = g.get(&cache_key) {
                return Ok(hit.result.clone());
            }
        }

        let args = build_probe_args(
            &initial_cls.normalized_url,
            scope,
            bins,
            proxy_url.as_deref(),
            &cookies,
        );
        let (code, stdout, stderr) =
            run_hidden_output(&bins.ytdlp_exe(), &args, ProcessPriority::BelowNormal)
                .await
                .map_err(|e| JobError {
                    code: "spawn_error".to_string(),
                    message_pt: tr!(
                    "Não foi possível iniciar o yt-dlp: {e}",
                    "Couldn't start yt-dlp: {e}",
                    "No se pudo iniciar yt-dlp: {e}"
                ),
                    hint_pt: None,
                    action: crate::errors::ErrorAction::Retry,
                    transient: true,
                    suggest_impersonate: false,
                })?;

        if code != 0 || stdout.trim().is_empty() {
            return Err(map_stderr_to_job_error(&stderr, &initial_cls.platform_id, "C:"));
        }

        // If watch?v=...&list=... and scope is SingleVideo, optionally grab flat playlist count in parallel/quick follow-up
        let mut sibling_playlist_count: Option<usize> = None;
        if initial_cls.has_video_and_playlist && scope == ProbeScope::SingleVideo {
            if let Some(list_url) = extract_playlist_only_url(&initial_cls.normalized_url) {
                let p_args = build_probe_args(
                    &list_url,
                    ProbeScope::FullPlaylist,
                    bins,
                    proxy_url.as_deref(),
                    &cookies,
                );
                if let Ok((0, p_out, _)) =
                    run_hidden_output(&bins.ytdlp_exe(), &p_args, ProcessPriority::BelowNormal).await
                {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&p_out) {
                        sibling_playlist_count = v
                            .get("playlist_count")
                            .and_then(|n| n.as_u64())
                            .map(|n| n as usize)
                            .or_else(|| {
                                v.get("entries")
                                    .and_then(|e| e.as_array())
                                    .map(|a| a.len())
                            });
                    }
                }
            }
        }

        let json: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|e| JobError {
            code: "parse_error".to_string(),
            message_pt: tr!(
                    "Resposta inesperada na leitura do link: {e}",
                    "Unexpected response while reading the link: {e}",
                    "Respuesta inesperada al leer el enlace: {e}"
                ),
            hint_pt: None,
            action: crate::errors::ErrorAction::UpdateYtdlp,
            transient: false,
            suggest_impersonate: false,
        })?;

        let mut result = parse_probe_json(&initial_cls, &json, sibling_playlist_count).await;

        // Cache thumbnail on local disk and rewrite URL to boladaomedia:// custom protocol (CSP safe + offline)
        if let Some(remote_thumb) = result.thumb_url.clone() {
            if let Some(local_proto_url) = cache_thumbnail_to_disk(&remote_thumb, proxy_url.as_deref()).await {
                result.thumb_url = Some(local_proto_url);
            }
        }

        self.cache.lock().insert(
            cache_key,
            CachedProbe {
                created_at: Instant::now(),
                result: result.clone(),
            },
        );
        Ok(result)
    }
}

fn build_probe_args(
    url: &str,
    scope: ProbeScope,
    bins: &BinManager,
    proxy_url: Option<&str>,
    cookies: &CookieSource,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("--ignore-config"),
        OsString::from("--encoding"),
        OsString::from("utf-8"),
        OsString::from("--color"),
        OsString::from("never"),
        OsString::from("--no-warnings"),
        OsString::from("-J"),
        OsString::from("--flat-playlist"),
        OsString::from("--ffmpeg-location"),
        bins.ffmpeg_dir().into_os_string(),
        OsString::from("--js-runtimes"),
        OsString::from(format!("deno:{}", bins.deno_exe().display())),
    ];

    match scope {
        ProbeScope::SingleVideo => args.push(OsString::from("--no-playlist")),
        ProbeScope::FullPlaylist => args.push(OsString::from("--yes-playlist")),
    }

    if let Some(proxy) = proxy_url {
        args.push(OsString::from("--proxy"));
        args.push(OsString::from(proxy));
    }

    match cookies {
        CookieSource::None => {}
        CookieSource::File(p) => {
            args.push(OsString::from("--cookies"));
            args.push(p.as_os_str().to_os_string());
        }
        CookieSource::FirefoxBrowser => {
            args.push(OsString::from("--cookies-from-browser"));
            args.push(OsString::from("firefox"));
        }
    }

    args.push(OsString::from("--"));
    args.push(OsString::from(url));
    args
}

fn extract_playlist_only_url(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw).ok()?;
    let list_id = parsed
        .query_pairs()
        .find(|(k, _)| k == "list")
        .map(|(_, v)| v.into_owned())?;
    let host = parsed.host_str().unwrap_or("www.youtube.com");
    Some(format!("https://{host}/playlist?list={list_id}"))
}

async fn parse_probe_json(
    initial_cls: &Classification,
    v: &serde_json::Value,
    sibling_playlist_count: Option<usize>,
) -> ProbeResult {
    let extractor_key = v
        .get("extractor_key")
        .or_else(|| v.get("ie_key"))
        .and_then(|s| s.as_str())
        .unwrap_or("Generic")
        .to_string();

    let raw_entries = v.get("entries").and_then(|e| e.as_array());
    let is_playlist = v.get("_type").and_then(|t| t.as_str()) == Some("playlist")
        || raw_entries.map(|a| !a.is_empty()).unwrap_or(false);

    let mut entries = Vec::new();
    if let Some(arr) = raw_entries {
        for (idx, item) in arr.iter().enumerate() {
            if item.is_null() {
                continue;
            }
            let id = item
                .get("id")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string();
            let item_url = item
                .get("url")
                .or_else(|| item.get("webpage_url"))
                .and_then(|s| s.as_str())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| {
                    if !id.is_empty() && initial_cls.platform_id.starts_with("youtube") {
                        format!("https://www.youtube.com/watch?v={id}")
                    } else {
                        initial_cls.normalized_url.clone()
                    }
                });
            let title = item
                .get("title")
                .and_then(|s| s.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| tr!("Item sem título", "Untitled item", "Elemento sin título"));
            let duration_s = item.get("duration").and_then(|n| n.as_f64());
            let live_status = item.get("live_status").and_then(|s| s.as_str()).unwrap_or("");
            let is_live = item.get("is_live").and_then(|b| b.as_bool()).unwrap_or(false)
                || live_status == "is_live"
                || live_status == "is_upcoming";
            let is_short = item_url.contains("/shorts/")
                || duration_s.map(|d| d > 0.0 && d <= 60.0).unwrap_or(false);
            let thumb_url = pick_best_thumb(item);
            let uploader = item
                .get("channel")
                .or_else(|| item.get("uploader"))
                .or_else(|| item.get("artist"))
                .and_then(|s| s.as_str())
                .map(ToOwned::to_owned);

            entries.push(PlaylistEntryPreview {
                index: (idx + 1) as u32,
                id,
                url: item_url,
                title,
                uploader,
                duration_s,
                thumb_url,
                is_short,
                is_live,
            });
        }
    }

    let playlist_count = if is_playlist {
        Some(
            v.get("playlist_count")
                .and_then(|n| n.as_u64())
                .map(|n| n as usize)
                .unwrap_or(entries.len()),
        )
    } else {
        sibling_playlist_count
    };

    let cls = refine_with_extractor_key(
        initial_cls.clone(),
        &extractor_key,
        if is_playlist { playlist_count } else { None },
    );

    let id = v
        .get("id")
        .and_then(|s| s.as_str())
        .unwrap_or("midia")
        .to_string();
    let raw_title = v
        .get("title")
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let collapsed_title = raw_title.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = if collapsed_title.is_empty() {
        tr!("Sem título", "Untitled", "Sin título")
    } else {
        collapsed_title
    };
    let channel_or_artist = v
        .get("artist")
        .or_else(|| v.get("channel"))
        .or_else(|| v.get("uploader"))
        .or_else(|| v.get("creator"))
        .and_then(|s| s.as_str())
        .unwrap_or(&cls.platform_name)
        .to_string();

    let upload_date = v
        .get("upload_date")
        .and_then(|s| s.as_str())
        .and_then(|d| {
            if d.len() == 8 {
                Some(format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8]))
            } else {
                None
            }
        });

    let duration_s = v.get("duration").and_then(|n| n.as_f64());
    let thumb_url = pick_best_thumb(v);

    // Inspect real formats for resolutions, HDR, H.264 availability, audio languages & sizes
    let mut best_audio_bytes: Option<u64> = None;
    let mut best_audio_acodec: Option<String> = None;
    let mut audio_langs = Vec::new();
    let mut res_buckets: HashMap<(u32, u32), VideoFormatOption> = HashMap::new();
    let mut saw_portrait_video = match (
        v.get("width").and_then(|n| n.as_u64()),
        v.get("height").and_then(|n| n.as_u64()),
    ) {
        (Some(w), Some(h)) if w > 0 && h > w => true,
        _ => false,
    };

    if let Some(formats) = v.get("formats").and_then(|f| f.as_array()) {
        for f in formats {
            let vcodec_opt = f.get("vcodec").and_then(|s| s.as_str());
            let acodec = f.get("acodec").and_then(|s| s.as_str()).unwrap_or("none");
            let ext = f.get("ext").and_then(|s| s.as_str()).unwrap_or("");
            let width = f.get("width").and_then(|n| n.as_u64()).map(|n| n as u32);
            let height = f.get("height").and_then(|n| n.as_u64()).map(|n| n as u32);
            let size = f
                .get("filesize")
                .or_else(|| f.get("filesize_approx"))
                .and_then(|n| n.as_u64());

            let explicitly_no_video = vcodec_opt == Some("none");

            if explicitly_no_video && acodec != "none" {
                if let Some(lang) = f.get("language").and_then(|s| s.as_str()) {
                    let clean = lang.trim().to_string();
                    if !clean.is_empty() && !audio_langs.contains(&clean) {
                        audio_langs.push(clean);
                    }
                }
                if let Some(sz) = size {
                    if best_audio_bytes.map_or(true, |prev| sz > prev) {
                        best_audio_bytes = Some(sz);
                        best_audio_acodec = Some(acodec.to_string());
                    }
                } else if best_audio_acodec.is_none() {
                    best_audio_acodec = Some(acodec.to_string());
                }
            }

            if !explicitly_no_video {
                // In yt-dlp (`-S res:<N>`), resolution is defined as `min(width, height)` so vertical
                // 9:16 videos (1080x1920 Reels/Shorts/X) report 1080p rather than 1920p!
                let res_dim = match (width, height) {
                    (Some(w), Some(h)) if w > 0 && h > 0 => {
                        if h > w {
                            saw_portrait_video = true;
                        }
                        w.min(h)
                    }
                    (_, Some(h)) if h > 0 => h,
                    (Some(w), _) if w > 0 => w,
                    _ => continue,
                };
                if res_dim < 144 {
                    continue;
                }
                let raw_fps = f.get("fps").and_then(|n| n.as_f64()).unwrap_or(30.0);
                let fps_tier = if raw_fps >= 50.0 { raw_fps.round() as u32 } else { 30 };
                let dyn_range = f
                    .get("dynamic_range")
                    .and_then(|s| s.as_str())
                    .unwrap_or("SDR");
                let is_hdr = dyn_range != "SDR" && !dyn_range.is_empty();

                let vcodec_raw = vcodec_opt.unwrap_or("unknown");
                // Progressive MP4 streams on Instagram Reels and X.com frequently have `vcodec: null` or `"unknown"` with `ext: "mp4"` (H.264)!
                let is_h264 = vcodec_raw.starts_with("avc1")
                    || vcodec_raw.contains("h264")
                    || ((vcodec_raw == "unknown" || vcodec_raw.is_empty()) && ext.eq_ignore_ascii_case("mp4"));
                let codec_short = if is_h264 {
                    "h264"
                } else if vcodec_raw.starts_with("vp9") || vcodec_raw.starts_with("vp09") {
                    "vp9"
                } else if vcodec_raw.starts_with("av01") {
                    "av1"
                } else if vcodec_raw.starts_with("hev1") || vcodec_raw.starts_with("hvc1") {
                    "hevc"
                } else {
                    vcodec_raw
                };

                let label = if fps_tier >= 50 {
                    format!("{res_dim}p{fps_tier}")
                } else {
                    format!("{res_dim}p")
                };

                let entry = res_buckets
                    .entry((res_dim, fps_tier))
                    .or_insert_with(|| VideoFormatOption {
                        height: res_dim,
                        fps: fps_tier,
                        label,
                        has_hdr: false,
                        has_h264: false,
                        vcodecs: Vec::new(),
                        estimated_total_bytes: None,
                    });
                entry.has_hdr |= is_hdr;
                entry.has_h264 |= is_h264;
                if !entry.vcodecs.iter().any(|c| c == codec_short) {
                    entry.vcodecs.push(codec_short.to_string());
                }
                if let Some(sz) = size {
                    let combined = if acodec == "none" {
                        sz + best_audio_bytes.unwrap_or(0)
                    } else {
                        sz
                    };
                    entry.estimated_total_bytes = Some(
                        entry
                            .estimated_total_bytes
                            .map_or(combined, |prev| prev.max(combined)),
                    );
                }
            }
        }
    }

    let mut video_resolutions: Vec<VideoFormatOption> = res_buckets.into_values().collect();
    video_resolutions.sort_by(|a, b| b.height.cmp(&a.height).then_with(|| b.fps.cmp(&a.fps)));

    // Subtitles (manual vs automatic)
    let mut manual_subtitles = Vec::new();
    if let Some(subs) = v.get("subtitles").and_then(|o| o.as_object()) {
        for (lang, tracks) in subs {
            if lang == "live_chat" {
                continue;
            }
            let name = tracks
                .as_array()
                .and_then(|a| a.first())
                .and_then(|t| t.get("name"))
                .and_then(|s| s.as_str())
                .unwrap_or(lang)
                .to_string();
            manual_subtitles.push(SubtitleTrackInfo {
                lang: lang.clone(),
                name,
                is_auto: false,
            });
        }
    }

    let mut auto_subtitles = Vec::new();
    if let Some(subs) = v.get("automatic_captions").and_then(|o| o.as_object()) {
        for (lang, tracks) in subs {
            if !matches!(
                lang.as_str(),
                "pt" | "pt-BR" | "pt-PT" | "en" | "es" | "ja" | "fr" | "de"
            ) && auto_subtitles.len() >= 12
            {
                continue;
            }
            let name = tracks
                .as_array()
                .and_then(|a| a.first())
                .and_then(|t| t.get("name"))
                .and_then(|s| s.as_str())
                .unwrap_or(lang)
                .to_string();
            auto_subtitles.push(SubtitleTrackInfo {
                lang: lang.clone(),
                name,
                is_auto: true,
            });
        }
    }

    let mut chapters = Vec::new();
    if let Some(ch_arr) = v.get("chapters").and_then(|a| a.as_array()) {
        for ch in ch_arr {
            let title = ch
                .get("title")
                .and_then(|s| s.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| tr!("Capítulo", "Chapter", "Capítulo"));
            let start_time = ch.get("start_time").and_then(|n| n.as_f64()).unwrap_or(0.0);
            let end_time = ch.get("end_time").and_then(|n| n.as_f64()).unwrap_or(start_time);
            chapters.push(ChapterInfo {
                title,
                start_time,
                end_time,
            });
        }
    }

    let live_status = v.get("live_status").and_then(|s| s.as_str()).unwrap_or("");
    let is_live = v.get("is_live").and_then(|b| b.as_bool()).unwrap_or(false)
        || live_status == "is_live";
    let is_upcoming = live_status == "is_upcoming";
    let release_timestamp = v.get("release_timestamp").and_then(|n| n.as_i64());

    let playlist_title = if is_playlist {
        Some(title.clone())
    } else {
        v.get("playlist_title")
            .and_then(|s| s.as_str())
            .map(ToOwned::to_owned)
    };

    let mut final_cls = cls;
    if is_live && !matches!(final_cls.kind, SourceKind::Playlist) {
        final_cls.kind = SourceKind::Live;
    }

    ProbeResult {
        url: initial_cls.normalized_url.clone(),
        classification: final_cls,
        extractor_key,
        id,
        title,
        channel_or_artist,
        upload_date,
        duration_s,
        thumb_url,
        is_portrait: saw_portrait_video,
        is_playlist,
        playlist_title,
        playlist_count,
        entries,
        video_resolutions,
        best_audio_bytes,
        best_audio_acodec,
        audio_languages: audio_langs,
        manual_subtitles,
        auto_subtitles,
        chapters,
        is_live,
        is_upcoming,
        release_timestamp,
    }
}

fn pick_best_thumb(v: &serde_json::Value) -> Option<String> {
    if let Some(s) = v.get("thumbnail").and_then(|t| t.as_str()) {
        if s.starts_with("http") {
            return Some(s.to_string());
        }
    }
    if let Some(arr) = v.get("thumbnails").and_then(|t| t.as_array()) {
        for item in arr.iter().rev() {
            if let Some(u) = item.get("url").and_then(|s| s.as_str()) {
                if u.starts_with("http") {
                    return Some(u.to_string());
                }
            }
        }
    }
    None
}

pub fn thumbs_cache_dir() -> PathBuf {
    let dir = app_local_dir().join("cache").join("thumbs");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Downloads a remote thumbnail once into `%LOCALAPPDATA%\Boladao\cache\thumbs\` and returns
/// `http://boladaomedia.localhost/thumb/<hash>.img` so strict CSP works without external domains.
pub async fn cache_thumbnail_to_disk(remote_url: &str, proxy_url: Option<&str>) -> Option<String> {
    if remote_url.starts_with(&crate::paths::local_media_url("thumb", "")) {
        return Some(remote_url.to_string());
    }
    let mut hasher = Sha256::new();
    hasher.update(remote_url.as_bytes());
    let key = hex::encode(hasher.finalize());
    let fname = format!("{key}.img");
    let file_path = thumbs_cache_dir().join(&fname);

    if file_path.is_file() {
        return Some(crate::paths::local_media_url("thumb", &fname));
    }

    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("Mozilla/5.0");
    if let Some(p) = proxy_url {
        if let Ok(px) = reqwest::Proxy::all(p) {
            builder = builder.proxy(px);
        }
    }
    let client = builder.build().ok()?;
    let bytes = client.get(remote_url).send().await.ok()?.bytes().await.ok()?;
    if bytes.is_empty() {
        return None;
    }
    fs::write(&file_path, &bytes).ok()?;
    Some(crate::paths::local_media_url("thumb", &fname))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_instagram_reel_and_x_vertical_formats_cleanly() {
        let ig_cls = classify_url("https://www.instagram.com/reel/Df2rFlJsSk0/").unwrap();
        // Mirrors real yt-dlp JSON for Instagram Reels (progressive MP4 with vcodec=null + DASH VP9 1080x1920)
        let ig_json = serde_json::json!({
            "id": "Df2rFlJsSk0",
            "extractor_key": "Instagram",
            "title": "Video by backyardmorning\n#chickens #morning",
            "uploader": "backyardmorning",
            "width": 1080,
            "height": 1920,
            "formats": [
                {
                    "format_id": "dash-audio",
                    "ext": "m4a",
                    "vcodec": "none",
                    "acodec": "mp4a.40.2",
                    "filesize": 200000
                },
                {
                    "format_id": "dash-vp9-1080",
                    "ext": "mp4",
                    "width": 1080,
                    "height": 1920,
                    "vcodec": "vp9",
                    "acodec": "none",
                    "filesize": 2500000
                },
                {
                    "format_id": "0",
                    "ext": "mp4",
                    "width": 1080,
                    "height": 1920,
                    "vcodec": null,
                    "acodec": null
                },
                {
                    "format_id": "dash-vp9-720",
                    "ext": "mp4",
                    "width": 720,
                    "height": 1280,
                    "vcodec": "vp9",
                    "acodec": "none"
                }
            ]
        });

        let res = parse_probe_json(&ig_cls, &ig_json, None).await;
        assert!(res.is_portrait);
        assert_eq!(res.title, "Video by backyardmorning #chickens #morning");
        // Short side is 1080p & 720p (NOT 1920p / 1280p), and progressive MP4 sets has_h264 = true on 1080p!
        assert_eq!(res.video_resolutions[0].height, 1080);
        assert!(res.video_resolutions[0].has_h264);
        assert_eq!(res.video_resolutions[1].height, 720);

        let x_cls = classify_url("https://x.com/user/status/1850000000000").unwrap();
        let x_json = serde_json::json!({
            "id": "1850000000000",
            "extractor_key": "Twitter",
            "title": "User on X: \"linha 1\n\nlinha 2 com texto enorme\" / X",
            "formats": [
                {
                    "format_id": "http-2176",
                    "ext": "mp4",
                    "width": 720,
                    "height": 1280,
                    "vcodec": null,
                    "acodec": null
                }
            ]
        });
        let x_res = parse_probe_json(&x_cls, &x_json, None).await;
        assert!(x_res.is_portrait);
        assert_eq!(x_res.video_resolutions.len(), 1);
        assert_eq!(x_res.video_resolutions[0].height, 720);
        assert!(x_res.video_resolutions[0].has_h264);
    }
}
