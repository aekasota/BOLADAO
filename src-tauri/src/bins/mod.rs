use crate::paths::app_local_dir;
use crate::winproc::{run_hidden_output, ProcessPriority};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

include!(concat!(env!("OUT_DIR"), "/embedded_manifest.rs"));

static YTDLP_TAR_ZST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ytdlp.tar.zst"));
static DENO_TAR_ZST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/deno.tar.zst"));
static FFMPEG_TAR_ZST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ffmpeg.tar.zst"));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledManifest {
    pub key: String,
    pub version: String,
    pub archive_sha256: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub main_exe_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FfmpegCapabilities {
    pub ffmpeg_version: String,
    pub has_whisper: bool,
    pub has_rubberband: bool,
    pub has_soxr: bool,
    pub has_libass: bool,
    pub has_loudnorm: bool,
    pub has_silenceremove: bool,
    pub has_showwavespic: bool,
    pub has_showspectrumpic: bool,
    pub has_palettegen: bool,
    pub hw_encoders: Vec<String>,
    pub sw_encoders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinsStatusPayload {
    pub ready: bool,
    pub extracting: bool,
    pub stage: String,
    pub component: Option<String>,
    pub progress_pct: f32,
    pub ytdlp_version: String,
    pub ytdlp_previous_version: Option<String>,
    pub ytdlp_channel: String,
    pub deno_version: String,
    pub ffmpeg_version: String,
    pub update_available: Option<String>,
    pub capabilities: FfmpegCapabilities,
    pub error: Option<String>,
}

pub struct BinManager {
    root_dir: PathBuf,
    state: RwLock<BinsStatusPayload>,
}

impl BinManager {
    pub fn new() -> Arc<Self> {
        let root_dir = app_local_dir().join("bin");
        let _ = fs::create_dir_all(&root_dir);
        let initial = BinsStatusPayload {
            ready: false,
            extracting: false,
            stage: "Verificando motores...".to_string(),
            component: None,
            progress_pct: 0.0,
            ytdlp_version: EMBEDDED_YTDLP_META.version.to_string(),
            ytdlp_previous_version: None,
            ytdlp_channel: "stable".to_string(),
            deno_version: EMBEDDED_DENO_META.version.to_string(),
            ffmpeg_version: EMBEDDED_FFMPEG_META.version.to_string(),
            update_available: None,
            capabilities: FfmpegCapabilities::default(),
            error: None,
        };
        Arc::new(Self {
            root_dir,
            state: RwLock::new(initial),
        })
    }

    pub fn status(&self) -> BinsStatusPayload {
        self.state.read().clone()
    }

    pub fn ytdlp_exe(&self) -> PathBuf {
        let current_ptr = self.root_dir.join("ytdlp").join("current.txt");
        if let Ok(ver) = fs::read_to_string(&current_ptr) {
            let ver = ver.trim();
            if !ver.is_empty() {
                let versioned = self
                    .root_dir
                    .join("ytdlp")
                    .join("versions")
                    .join(ver)
                    .join("yt-dlp.exe");
                if versioned.is_file() {
                    return versioned;
                }
            }
        }
        self.root_dir
            .join("ytdlp")
            .join("versions")
            .join(EMBEDDED_YTDLP_META.version)
            .join("yt-dlp.exe")
    }

    pub fn deno_exe(&self) -> PathBuf {
        self.root_dir.join("deno").join("deno.exe")
    }

    pub fn ffmpeg_dir(&self) -> PathBuf {
        self.root_dir.join("ffmpeg")
    }

    pub fn ffmpeg_exe(&self) -> PathBuf {
        self.ffmpeg_dir().join("ffmpeg.exe")
    }

    pub fn ffprobe_exe(&self) -> PathBuf {
        self.ffmpeg_dir().join("ffprobe.exe")
    }

    fn emit_state(&self, app: Option<&AppHandle>) {
        if let Some(h) = app {
            let snapshot = self.state.read().clone();
            let _ = h.emit("bins:status", snapshot);
        }
    }

    /// Checks whether every component is already extracted and valid without hashing 360 MB on boot.
    pub fn needs_extraction(&self) -> bool {
        !self.is_ytdlp_ready()
            || !self.is_simple_component_ready(&EMBEDDED_DENO_META, "deno.exe")
            || !self.is_simple_component_ready(&EMBEDDED_FFMPEG_META, "ffmpeg.exe")
    }

    fn is_simple_component_ready(&self, meta: &EmbeddedComponentMeta, main_exe: &str) -> bool {
        let dir = self.root_dir.join(meta.key);
        let manifest_path = dir.join(".manifest.json");
        let exe_path = dir.join(main_exe);
        if !manifest_path.is_file() || !exe_path.is_file() {
            return false;
        }
        let Ok(raw) = fs::read_to_string(&manifest_path) else {
            return false;
        };
        let Ok(installed) = serde_json::from_str::<InstalledManifest>(&raw) else {
            return false;
        };
        let Ok(exe_meta) = fs::metadata(&exe_path) else {
            return false;
        };
        installed.version == meta.version
            && installed.file_count == meta.file_count
            && installed.total_bytes == meta.total_bytes
            && exe_meta.len() == installed.main_exe_bytes
    }

    fn is_ytdlp_ready(&self) -> bool {
        let base = self.root_dir.join("ytdlp");
        let current_ptr = base.join("current.txt");
        if let Ok(cur_ver) = fs::read_to_string(&current_ptr) {
            let cur_ver = cur_ver.trim();
            if !cur_ver.is_empty() {
                let exe = base.join("versions").join(cur_ver).join("yt-dlp.exe");
                // Never overwrite a newer user-updated yt-dlp!
                if exe.is_file() && compare_versions(cur_ver, EMBEDDED_YTDLP_META.version) >= 0 {
                    return true;
                }
            }
        }
        false
    }

    /// Ensures all embedded binaries are extracted atomically and FFmpeg capabilities are probed.
    pub async fn ensure_ready(&self, app: Option<AppHandle>, ytdlp_channel: String) -> Result<(), String> {
        {
            let mut st = self.state.write();
            st.ytdlp_channel = ytdlp_channel;
        }

        let need_ytdlp = !self.is_ytdlp_ready();
        let need_deno = !self.is_simple_component_ready(&EMBEDDED_DENO_META, "deno.exe");
        let need_ffmpeg = !self.is_simple_component_ready(&EMBEDDED_FFMPEG_META, "ffmpeg.exe");

        if need_ytdlp || need_deno || need_ffmpeg {
            {
                let mut st = self.state.write();
                st.extracting = true;
                st.ready = false;
                st.stage = "Preparando os motores...".to_string();
                st.progress_pct = 0.02;
            }
            self.emit_state(app.as_ref());

            let total_to_extract = (if need_ytdlp { EMBEDDED_YTDLP_META.total_bytes } else { 0 })
                + (if need_deno { EMBEDDED_DENO_META.total_bytes } else { 0 })
                + (if need_ffmpeg { EMBEDDED_FFMPEG_META.total_bytes } else { 0 });
            let mut extracted_so_far = 0u64;

            let root_clone = self.root_dir.clone();
            let app_clone = app.clone();
            // Perform heavy zstd+tar extraction on a blocking thread with live progress updates
            let manager_ptr = self as *const BinManager as usize;

            tokio::task::spawn_blocking(move || -> Result<(), String> {
                let mgr = unsafe { &*(manager_ptr as *const BinManager) };

                if need_ytdlp {
                    let target = root_clone
                        .join("ytdlp")
                        .join("versions")
                        .join(EMBEDDED_YTDLP_META.version);
                    extract_tar_zst_atomic(
                        YTDLP_TAR_ZST,
                        &EMBEDDED_YTDLP_META,
                        &target,
                        "yt-dlp.exe",
                        "Extraindo yt-dlp...",
                        &mut extracted_so_far,
                        total_to_extract,
                        mgr,
                        app_clone.as_ref(),
                    )?;
                    let ytdlp_base = root_clone.join("ytdlp");
                    let _ = fs::create_dir_all(&ytdlp_base);
                    write_atomic_text(&ytdlp_base.join("current.txt"), EMBEDDED_YTDLP_META.version)?;
                }

                if need_deno {
                    let target = root_clone.join("deno");
                    extract_tar_zst_atomic(
                        DENO_TAR_ZST,
                        &EMBEDDED_DENO_META,
                        &target,
                        "deno.exe",
                        "Extraindo runtime Deno...",
                        &mut extracted_so_far,
                        total_to_extract,
                        mgr,
                        app_clone.as_ref(),
                    )?;
                }

                if need_ffmpeg {
                    let target = root_clone.join("ffmpeg");
                    extract_tar_zst_atomic(
                        FFMPEG_TAR_ZST,
                        &EMBEDDED_FFMPEG_META,
                        &target,
                        "ffmpeg.exe",
                        "Extraindo FFmpeg e FFprobe...",
                        &mut extracted_so_far,
                        total_to_extract,
                        mgr,
                        app_clone.as_ref(),
                    )?;
                }

                Ok(())
            })
            .await
            .map_err(|e| format!("Falha interna na extração: {e}"))??;
        }

        // Read installed versions & rollback availability
        let current_ytdlp = fs::read_to_string(self.root_dir.join("ytdlp").join("current.txt"))
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| EMBEDDED_YTDLP_META.version.to_string());

        let prev_ytdlp = fs::read_to_string(self.root_dir.join("ytdlp").join("previous.txt"))
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| {
                !s.is_empty()
                    && *s != current_ytdlp
                    && self
                        .root_dir
                        .join("ytdlp")
                        .join("versions")
                        .join(s)
                        .join("yt-dlp.exe")
                        .is_file()
            });

        {
            let mut st = self.state.write();
            st.stage = "Detectando recursos de vídeo e áudio...".to_string();
            st.progress_pct = 0.94;
            st.ytdlp_version = current_ytdlp;
            st.ytdlp_previous_version = prev_ytdlp;
        }
        self.emit_state(app.as_ref());

        let caps = self.load_or_probe_ffmpeg_capabilities().await;

        {
            let mut st = self.state.write();
            st.capabilities = caps;
            st.extracting = false;
            st.ready = true;
            st.stage = "Pronto".to_string();
            st.progress_pct = 1.0;
            st.error = None;
        }
        self.emit_state(app.as_ref());
        Ok(())
    }

    async fn load_or_probe_ffmpeg_capabilities(&self) -> FfmpegCapabilities {
        let cache_file = self.ffmpeg_dir().join(".capabilities.json");
        if let Ok(raw) = fs::read_to_string(&cache_file) {
            if let Ok(cached) = serde_json::from_str::<FfmpegCapabilities>(&raw) {
                if cached.ffmpeg_version == EMBEDDED_FFMPEG_META.version {
                    return cached;
                }
            }
        }

        let ff = self.ffmpeg_exe();
        let mut caps = FfmpegCapabilities {
            ffmpeg_version: EMBEDDED_FFMPEG_META.version.to_string(),
            ..Default::default()
        };

        if let Ok((_, stdout, stderr)) = run_hidden_output(
            &ff,
            &[OsString::from("-hide_banner"), OsString::from("-filters")],
            ProcessPriority::BelowNormal,
        )
        .await
        {
            let text = format!("{stdout}\n{stderr}");
            caps.has_whisper = has_token(&text, "whisper");
            caps.has_rubberband = has_token(&text, "rubberband");
            caps.has_soxr = has_token(&text, "aresample");
            caps.has_libass = has_token(&text, "subtitles") && has_token(&text, "ass");
            caps.has_loudnorm = has_token(&text, "loudnorm");
            caps.has_silenceremove = has_token(&text, "silenceremove");
            caps.has_showwavespic = has_token(&text, "showwavespic");
            caps.has_showspectrumpic = has_token(&text, "showspectrumpic");
            caps.has_palettegen = has_token(&text, "palettegen") && has_token(&text, "paletteuse");
        }

        if let Ok((_, stdout, stderr)) = run_hidden_output(
            &ff,
            &[OsString::from("-hide_banner"), OsString::from("-encoders")],
            ProcessPriority::BelowNormal,
        )
        .await
        {
            let text = format!("{stdout}\n{stderr}");
            for sw in ["libx264", "libx265", "libsvtav1", "libmp3lame", "libopus", "flac", "alac"]
            {
                if has_token(&text, sw) {
                    caps.sw_encoders.push(sw.to_string());
                }
            }

            // Section 8.2: Do NOT merely trust `-encoders` for GPU encoders; run a 0.1s test encode!
            let candidates = [
                "h264_nvenc",
                "hevc_nvenc",
                "av1_nvenc",
                "h264_qsv",
                "hevc_qsv",
                "av1_qsv",
                "h264_amf",
                "hevc_amf",
                "av1_amf",
            ];
            for enc in candidates {
                if !has_token(&text, enc) {
                    continue;
                }
                let args = vec![
                    OsString::from("-hide_banner"),
                    OsString::from("-loglevel"),
                    OsString::from("error"),
                    OsString::from("-f"),
                    OsString::from("lavfi"),
                    OsString::from("-i"),
                    OsString::from("color=s=256x256:d=0.1"),
                    OsString::from("-c:v"),
                    OsString::from(enc),
                    OsString::from("-f"),
                    OsString::from("null"),
                    OsString::from("-"),
                ];
                if let Ok((code, _, _)) =
                    run_hidden_output(&ff, &args, ProcessPriority::BelowNormal).await
                {
                    if code == 0 {
                        caps.hw_encoders.push(enc.to_string());
                    }
                }
            }
        }

        if let Ok(json) = serde_json::to_string_pretty(&caps) {
            let _ = fs::write(&cache_file, json);
        }
        caps
    }

    /// Runs `yt-dlp -v` proving FFmpeg + Deno JS runtime integration.
    pub async fn run_diagnostic(&self) -> Result<String, String> {
        let args = vec![
            OsString::from("-v"),
            OsString::from("--ignore-config"),
            OsString::from("--encoding"),
            OsString::from("utf-8"),
            OsString::from("--color"),
            OsString::from("never"),
            OsString::from("--ffmpeg-location"),
            self.ffmpeg_dir().into_os_string(),
            OsString::from("--js-runtimes"),
            OsString::from(format!("deno:{}", self.deno_exe().display())),
        ];
        let (_, stdout, stderr) =
            run_hidden_output(&self.ytdlp_exe(), &args, ProcessPriority::BelowNormal)
                .await
                .map_err(|e| format!("Falha ao executar yt-dlp -v: {e}"))?;
        let combined = format!("{stdout}\n{stderr}");
        let relevant: Vec<&str> = combined
            .lines()
            .filter(|l| l.starts_with("[debug]") || l.contains("version"))
            .collect();
        Ok(relevant.join("\n"))
    }

    /// Returns cached searchable list of extractors from `yt-dlp --list-extractors`.
    pub async fn list_supported_sites(&self) -> Result<Vec<String>, String> {
        let ver = self.state.read().ytdlp_version.clone();
        let cache_path = self
            .root_dir
            .join("ytdlp")
            .join(format!("extractors-{ver}.json"));
        if let Ok(raw) = fs::read_to_string(&cache_path) {
            if let Ok(list) = serde_json::from_str::<Vec<String>>(&raw) {
                return Ok(list);
            }
        }

        let args = vec![
            OsString::from("--ignore-config"),
            OsString::from("--list-extractors"),
        ];
        let (code, stdout, stderr) =
            run_hidden_output(&self.ytdlp_exe(), &args, ProcessPriority::BelowNormal)
                .await
                .map_err(|e| format!("Falha listando extratores: {e}"))?;
        if code != 0 {
            return Err(format!("yt-dlp retornou erro ({code}): {stderr}"));
        }

        let list: Vec<String> = stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.ends_with("(CURRENTLY BROKEN)"))
            .map(ToOwned::to_owned)
            .collect();

        if let Ok(json) = serde_json::to_string(&list) {
            let _ = fs::write(&cache_path, json);
        }
        Ok(list)
    }

    /// Silent check against GitHub Releases (`stable` or `nightly`).
    pub async fn check_ytdlp_update(&self, channel: &str) -> Result<Option<String>, String> {
        let repo = if channel == "nightly" {
            "yt-dlp/yt-dlp-nightly-builds"
        } else {
            "yt-dlp/yt-dlp"
        };
        let url = format!("https://api.github.com/repos/{repo}/releases/latest");
        let client = reqwest::Client::builder()
            .user_agent("Boladao-Desktop/1.0")
            .build()
            .map_err(|e| e.to_string())?;

        #[derive(Deserialize)]
        struct GhRelease {
            tag_name: String,
        }

        let rel: GhRelease = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Sem conexão para checar atualização: {e}"))?
            .error_for_status()
            .map_err(|e| format!("GitHub API respondeu com erro: {e}"))?
            .json()
            .await
            .map_err(|e| format!("Resposta inválida do GitHub: {e}"))?;

        let cur = self.state.read().ytdlp_version.clone();
        if compare_versions(&rel.tag_name, &cur) > 0 {
            self.state.write().update_available = Some(rel.tag_name.clone());
            Ok(Some(rel.tag_name))
        } else {
            self.state.write().update_available = None;
            Ok(None)
        }
    }

    /// Updates `yt-dlp` (unpackaged `yt-dlp_win.zip`), verifies `SHA2-256SUMS`, preserves previous version for rollback, and swaps atomically.
    pub async fn update_ytdlp(
        &self,
        channel: &str,
        app: Option<&AppHandle>,
    ) -> Result<String, String> {
        let repo = if channel == "nightly" {
            "yt-dlp/yt-dlp-nightly-builds"
        } else {
            "yt-dlp/yt-dlp"
        };
        let api_url = format!("https://api.github.com/repos/{repo}/releases/latest");
        let client = reqwest::Client::builder()
            .user_agent("Boladao-Desktop/1.0")
            .build()
            .map_err(|e| e.to_string())?;

        #[derive(Deserialize)]
        struct GhAsset {
            name: String,
            browser_download_url: String,
        }
        #[derive(Deserialize)]
        struct GhRelease {
            tag_name: String,
            assets: Vec<GhAsset>,
        }

        let rel: GhRelease = client
            .get(&api_url)
            .send()
            .await
            .map_err(|e| format!("Erro consultando GitHub: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Erro na API do GitHub: {e}"))?
            .json()
            .await
            .map_err(|e| format!("JSON inválido do GitHub: {e}"))?;

        let zip_asset = rel
            .assets
            .iter()
            .find(|a| a.name == "yt-dlp_win.zip")
            .ok_or_else(|| "Asset yt-dlp_win.zip não encontrado na release.".to_string())?;
        let sums_asset = rel
            .assets
            .iter()
            .find(|a| a.name == "SHA2-256SUMS")
            .ok_or_else(|| "Arquivo SHA2-256SUMS não encontrado na release.".to_string())?;

        let sums_text = client
            .get(&sums_asset.browser_download_url)
            .send()
            .await
            .map_err(|e| format!("Falha baixando SHA2-256SUMS: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Erro HTTP em SHA2-256SUMS: {e}"))?
            .text()
            .await
            .map_err(|e| e.to_string())?;

        let expected_sha = sums_text
            .lines()
            .find_map(|line| {
                let mut parts = line.split_whitespace();
                let hash = parts.next()?;
                let fname = parts.next()?.trim_start_matches('*');
                if fname == "yt-dlp_win.zip" {
                    Some(hash.to_ascii_lowercase())
                } else {
                    None
                }
            })
            .ok_or_else(|| "Hash do yt-dlp_win.zip ausente em SHA2-256SUMS.".to_string())?;

        let zip_bytes = client
            .get(&zip_asset.browser_download_url)
            .send()
            .await
            .map_err(|e| format!("Falha baixando yt-dlp_win.zip: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Erro HTTP baixando yt-dlp_win.zip: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("Download interrompido: {e}"))?;

        let mut hasher = Sha256::new();
        hasher.update(&zip_bytes);
        let actual_sha = hex::encode(hasher.finalize());
        if actual_sha != expected_sha {
            return Err(format!(
                "Verificação SHA-256 falhou! Esperado {expected_sha}, recebido {actual_sha}."
            ));
        }

        let new_ver = rel.tag_name.clone();
        let versions_dir = self.root_dir.join("ytdlp").join("versions");
        let dest_dir = versions_dir.join(&new_ver);
        let tmp_dir = versions_dir.join(format!("{new_ver}.tmp-{}", std::process::id()));

        let zip_vec = zip_bytes.to_vec();
        let dest_clone = dest_dir.clone();
        let tmp_clone = tmp_dir.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            if tmp_clone.exists() {
                let _ = fs::remove_dir_all(&tmp_clone);
            }
            fs::create_dir_all(&tmp_clone).map_err(|e| e.to_string())?;
            let cursor = std::io::Cursor::new(zip_vec);
            let mut archive = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
            for i in 0..archive.len() {
                let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
                let Some(enclosed) = file.enclosed_name().map(|p| p.to_path_buf()) else {
                    continue;
                };
                let out_path = tmp_clone.join(&enclosed);
                if file.is_dir() {
                    fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
                } else {
                    if let Some(p) = out_path.parent() {
                        fs::create_dir_all(p).map_err(|e| e.to_string())?;
                    }
                    let mut out_file = File::create(&out_path).map_err(|e| e.to_string())?;
                    std::io::copy(&mut file, &mut out_file).map_err(|e| e.to_string())?;
                }
            }

            // Locate folder containing yt-dlp.exe in case zip had a wrapper subdir
            let actual_root = find_file_parent(&tmp_clone, "yt-dlp.exe")
                .ok_or_else(|| "yt-dlp.exe ausente no zip baixado.".to_string())?;

            if dest_clone.exists() {
                let _ = fs::remove_dir_all(&dest_clone);
            }
            if actual_root == tmp_clone {
                fs::rename(&tmp_clone, &dest_clone).map_err(|e| e.to_string())?;
            } else {
                fs::rename(&actual_root, &dest_clone).map_err(|e| e.to_string())?;
                let _ = fs::remove_dir_all(&tmp_clone);
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())??;

        let ytdlp_base = self.root_dir.join("ytdlp");
        let cur_ver = self.state.read().ytdlp_version.clone();
        if cur_ver != new_ver {
            let _ = write_atomic_text(&ytdlp_base.join("previous.txt"), &cur_ver);
        }
        write_atomic_text(&ytdlp_base.join("current.txt"), &new_ver)?;

        {
            let mut st = self.state.write();
            if cur_ver != new_ver {
                st.ytdlp_previous_version = Some(cur_ver);
            }
            st.ytdlp_version = new_ver.clone();
            st.ytdlp_channel = channel.to_string();
            st.update_available = None;
        }
        self.emit_state(app);
        Ok(new_ver)
    }

    /// Atomically rolls back `yt-dlp` to the previous saved version.
    pub fn rollback_ytdlp(&self, app: Option<&AppHandle>) -> Result<String, String> {
        let ytdlp_base = self.root_dir.join("ytdlp");
        let prev_path = ytdlp_base.join("previous.txt");
        let prev_ver = fs::read_to_string(&prev_path)
            .map_err(|_| "Nenhuma versão anterior salva para restaurar.".to_string())?;
        let prev_ver = prev_ver.trim().to_string();
        let prev_exe = ytdlp_base
            .join("versions")
            .join(&prev_ver)
            .join("yt-dlp.exe");
        if !prev_exe.is_file() {
            return Err(format!("Binário da versão anterior ({prev_ver}) não está mais no disco."));
        }

        write_atomic_text(&ytdlp_base.join("current.txt"), &prev_ver)?;
        let _ = fs::remove_file(&prev_path);

        {
            let mut st = self.state.write();
            st.ytdlp_version = prev_ver.clone();
            st.ytdlp_previous_version = None;
        }
        self.emit_state(app);
        Ok(prev_ver)
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_tar_zst_atomic(
    compressed: &[u8],
    meta: &EmbeddedComponentMeta,
    dest_dir: &Path,
    main_exe_name: &str,
    stage_label: &str,
    extracted_so_far: &mut u64,
    total_to_extract: u64,
    mgr: &BinManager,
    app: Option<&AppHandle>,
) -> Result<(), String> {
    let parent = dest_dir
        .parent()
        .ok_or_else(|| "Caminho inválido para extração".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("Erro criando pasta pai: {e}"))?;

    let sibling_tmp = parent.join(format!(
        "{}.extracting-{}",
        dest_dir.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    if sibling_tmp.exists() {
        let _ = fs::remove_dir_all(&sibling_tmp);
    }
    fs::create_dir_all(&sibling_tmp).map_err(|e| format!("Erro criando pasta temp: {e}"))?;

    {
        let mut st = mgr.state.write();
        st.stage = stage_label.to_string();
        st.component = Some(meta.key.to_string());
    }
    mgr.emit_state(app);

    let decoder = zstd::stream::read::Decoder::new(BufReader::new(compressed))
        .map_err(|e| format!("Erro abrindo zstd {}: {e}", meta.key))?;
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| format!("Erro lendo tar {}: {e}", meta.key))?;

    let mut last_emit_bytes = *extracted_so_far;
    for entry_res in entries {
        let mut entry = entry_res.map_err(|e| format!("Entrada tar corrompida: {e}"))?;
        let rel_path = entry
            .path()
            .map_err(|e| format!("Caminho tar inválido: {e}"))?
            .into_owned();
        let out_path = sibling_tmp.join(&rel_path);
        if let Some(p) = out_path.parent() {
            fs::create_dir_all(p).map_err(|e| format!("Erro criando diretório: {e}"))?;
        }
        let mut out_file =
            File::create(&out_path).map_err(|e| format!("Erro criando {}: {e}", out_path.display()))?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry
                .read(&mut buf)
                .map_err(|e| format!("Erro descomprimindo {}: {e}", meta.key))?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut out_file, &buf[..n])
                .map_err(|e| format!("Erro gravando em disco: {e}"))?;
            *extracted_so_far += n as u64;
            if *extracted_so_far - last_emit_bytes >= 3 * 1024 * 1024 {
                last_emit_bytes = *extracted_so_far;
                let pct = if total_to_extract > 0 {
                    0.04 + 0.88 * (*extracted_so_far as f32 / total_to_extract as f32)
                } else {
                    0.5
                };
                mgr.state.write().progress_pct = pct.min(0.92);
                mgr.emit_state(app);
            }
        }
    }

    let main_exe_path = sibling_tmp.join(main_exe_name);
    let main_exe_bytes = fs::metadata(&main_exe_path)
        .map_err(|e| format!("Executável principal {main_exe_name} ausente após extração: {e}"))?
        .len();

    let installed = InstalledManifest {
        key: meta.key.to_string(),
        version: meta.version.to_string(),
        archive_sha256: meta.archive_sha256.to_string(),
        file_count: meta.file_count,
        total_bytes: meta.total_bytes,
        main_exe_bytes,
    };
    let manifest_json = serde_json::to_string_pretty(&installed).map_err(|e| e.to_string())?;
    fs::write(sibling_tmp.join(".manifest.json"), manifest_json)
        .map_err(|e| format!("Erro gravando manifest: {e}"))?;

    if dest_dir.exists() {
        let backup = parent.join(format!(
            "{}.old-{}",
            dest_dir.file_name().unwrap().to_string_lossy(),
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&backup);
        fs::rename(dest_dir, &backup)
            .map_err(|e| format!("Binário em uso, não foi possível substituir {}: {e}", dest_dir.display()))?;
        fs::rename(&sibling_tmp, dest_dir)
            .map_err(|e| format!("Erro movendo pasta extraída: {e}"))?;
        let _ = fs::remove_dir_all(&backup);
    } else {
        fs::rename(&sibling_tmp, dest_dir)
            .map_err(|e| format!("Erro ativando pasta extraída: {e}"))?;
    }

    Ok(())
}

fn write_atomic_text(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, content).map_err(|e| format!("Erro escrevendo {}: {e}", tmp.display()))?;
    // `fs::rename` replaces the old file in one step; deleting it first would open a window in
    // which a crash leaves no file at all.
    fs::rename(&tmp, path).map_err(|e| format!("Erro movendo {}: {e}", path.display()))
}

fn find_file_parent(dir: &Path, target_file: &str) -> Option<PathBuf> {
    if dir.join(target_file).is_file() {
        return Some(dir.to_path_buf());
    }
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_file_parent(&p, target_file) {
                return Some(found);
            }
        }
    }
    None
}

fn has_token(text: &str, name: &str) -> bool {
    text.lines().any(|line| {
        line.split_whitespace().any(|tok| tok == name)
    })
}

/// Compares CalVer / SemVer version strings (`2026.11.01` vs `2026.10.26`, `v2.9.8` vs `2.9.7`).
pub fn compare_versions(a: &str, b: &str) -> i32 {
    let parse_nums = |s: &str| -> Vec<u64> {
        s.trim()
            .trim_start_matches('v')
            .split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse::<u64>().ok())
            .collect()
    };
    let va = parse_nums(a);
    let vb = parse_nums(b);
    let max_len = va.len().max(vb.len());
    for i in 0..max_len {
        let na = va.get(i).copied().unwrap_or(0);
        let nb = vb.get(i).copied().unwrap_or(0);
        if na > nb {
            return 1;
        }
        if na < nb {
            return -1;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_ytdlp_calver_correctly() {
        assert_eq!(compare_versions("2026.11.18", "2026.10.26"), 1);
        assert_eq!(compare_versions("2026.10.26", "2026.10.26"), 0);
        assert_eq!(compare_versions("2025.12.01", "2026.01.01"), -1);
        assert_eq!(compare_versions("2026.10.26.232815", "2026.10.26"), 1);
    }

    #[test]
    fn embedded_manifest_is_non_empty() {
        assert_eq!(EMBEDDED_MANIFEST.len(), 3);
        for item in EMBEDDED_MANIFEST {
            assert!(item.file_count >= 1);
            assert!(item.total_bytes > 1_000_000);
            assert!(item.compressed_bytes > 100_000);
        }
    }
}
