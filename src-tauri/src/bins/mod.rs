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

const YTDLP_EXE: &str = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
const DENO_EXE: &str = if cfg!(windows) { "deno.exe" } else { "deno" };
const FFMPEG_EXE: &str = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
const FFPROBE_EXE: &str = if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" };
/// FFmpeg's main executable inside its folder: the Linux build keeps its shared libraries in
/// `lib/`, next to `bin/` (the executables find them through `$ORIGIN/../lib`).
const FFMPEG_MAIN: &str = if cfg!(target_os = "linux") { "bin/ffmpeg" } else { FFMPEG_EXE };
/// yt-dlp release asset for this system: the "onedir" zips on Windows and Linux (they start
/// faster than the single-file builds), the single universal file on macOS.
const YTDLP_ASSET: &str = if cfg!(windows) {
    "yt-dlp_win.zip"
} else if cfg!(target_os = "macos") {
    "yt-dlp_macos"
} else {
    "yt-dlp_linux.zip"
};
/// Name of the executable inside the zip, renamed to `YTDLP_EXE` once installed.
const YTDLP_IN_ZIP: &str = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp_linux" };

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
        Self::with_root(app_local_dir().join("bin"))
    }

    /// Manager for engines kept in `root_dir` instead of the app folder (used by the tests).
    pub fn with_root(root_dir: PathBuf) -> Arc<Self> {
        let _ = fs::create_dir_all(&root_dir);
        let initial = BinsStatusPayload {
            ready: false,
            extracting: false,
            stage: tr!("Verificando motores...", "Checking the engines...", "Comprobando los motores..."),
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
                    .join(YTDLP_EXE);
                if versioned.is_file() {
                    return versioned;
                }
            }
        }
        self.root_dir
            .join("ytdlp")
            .join("versions")
            .join(EMBEDDED_YTDLP_META.version)
            .join(YTDLP_EXE)
    }

    pub fn deno_exe(&self) -> PathBuf {
        self.root_dir.join("deno").join(DENO_EXE)
    }

    /// Folder with `ffmpeg` and `ffprobe` (handed to yt-dlp as `--ffmpeg-location`).
    pub fn ffmpeg_dir(&self) -> PathBuf {
        let dir = self.root_dir.join("ffmpeg");
        if cfg!(target_os = "linux") {
            dir.join("bin")
        } else {
            dir
        }
    }

    pub fn ffmpeg_exe(&self) -> PathBuf {
        self.ffmpeg_dir().join(FFMPEG_EXE)
    }

    pub fn ffprobe_exe(&self) -> PathBuf {
        self.ffmpeg_dir().join(FFPROBE_EXE)
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
            || !self.is_simple_component_ready(&EMBEDDED_DENO_META, DENO_EXE)
            || !self.is_simple_component_ready(&EMBEDDED_FFMPEG_META, FFMPEG_MAIN)
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
                let exe = base.join("versions").join(cur_ver).join(YTDLP_EXE);
                // Never overwrite a newer user-updated yt-dlp!
                if exe.is_file() && compare_versions(cur_ver, EMBEDDED_YTDLP_META.version) >= 0 {
                    return true;
                }
            }
        }
        false
    }

    /// Ensures all embedded binaries are extracted atomically and FFmpeg capabilities are probed.
    pub async fn ensure_ready(self: &Arc<Self>, app: Option<AppHandle>, ytdlp_channel: String) -> Result<(), String> {
        {
            let mut st = self.state.write();
            st.ytdlp_channel = ytdlp_channel;
        }

        let need_ytdlp = !self.is_ytdlp_ready();
        let need_deno = !self.is_simple_component_ready(&EMBEDDED_DENO_META, DENO_EXE);
        let need_ffmpeg = !self.is_simple_component_ready(&EMBEDDED_FFMPEG_META, FFMPEG_MAIN);

        if need_ytdlp || need_deno || need_ffmpeg {
            {
                let mut st = self.state.write();
                st.extracting = true;
                st.ready = false;
                st.stage = tr!("Preparando os motores...", "Preparing the engines...", "Preparando los motores...");
                st.progress_pct = 0.02;
            }
            self.emit_state(app.as_ref());

            let total_to_extract = (if need_ytdlp { EMBEDDED_YTDLP_META.total_bytes } else { 0 })
                + (if need_deno { EMBEDDED_DENO_META.total_bytes } else { 0 })
                + (if need_ffmpeg { EMBEDDED_FFMPEG_META.total_bytes } else { 0 });
            let mut extracted_so_far = 0u64;

            let root_clone = self.root_dir.clone();
            let app_clone = app.clone();
            // Perform heavy zstd+tar extraction on a blocking thread with live progress updates.
            // The thread holds its own reference: it keeps running even if this future is dropped.
            let mgr = Arc::clone(self);

            tokio::task::spawn_blocking(move || -> Result<(), String> {
                if need_ytdlp {
                    let target = root_clone
                        .join("ytdlp")
                        .join("versions")
                        .join(EMBEDDED_YTDLP_META.version);
                    extract_tar_zst_atomic(
                        YTDLP_TAR_ZST,
                        &EMBEDDED_YTDLP_META,
                        &target,
                        YTDLP_EXE,
                        &tr!("Extraindo yt-dlp...", "Extracting yt-dlp...", "Extrayendo yt-dlp..."),
                        &mut extracted_so_far,
                        total_to_extract,
                        &mgr,
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
                        DENO_EXE,
                        &tr!("Extraindo runtime Deno...", "Extracting the Deno runtime...", "Extrayendo el runtime de Deno..."),
                        &mut extracted_so_far,
                        total_to_extract,
                        &mgr,
                        app_clone.as_ref(),
                    )?;
                }

                if need_ffmpeg {
                    let target = root_clone.join("ffmpeg");
                    extract_tar_zst_atomic(
                        FFMPEG_TAR_ZST,
                        &EMBEDDED_FFMPEG_META,
                        &target,
                        FFMPEG_MAIN,
                        &tr!("Extraindo FFmpeg e FFprobe...", "Extracting FFmpeg and FFprobe...", "Extrayendo FFmpeg y FFprobe..."),
                        &mut extracted_so_far,
                        total_to_extract,
                        &mgr,
                        app_clone.as_ref(),
                    )?;
                }

                Ok(())
            })
            .await
            .map_err(|e| tr!("Falha interna na extração: {e}", "Internal extraction failure: {e}", "Fallo interno en la extracción: {e}"))??;
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
                        .join(YTDLP_EXE)
                        .is_file()
            });

        {
            let mut st = self.state.write();
            st.stage = tr!("Detectando recursos de vídeo e áudio...", "Detecting video and audio features...", "Detectando funciones de video y audio...");
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
            caps.has_libass = has_token(&text, "subtitles") && has_token(&text, "ass");
            caps.has_loudnorm = has_token(&text, "loudnorm");
            caps.has_silenceremove = has_token(&text, "silenceremove");
            caps.has_showwavespic = has_token(&text, "showwavespic");
            caps.has_showspectrumpic = has_token(&text, "showspectrumpic");
            caps.has_palettegen = has_token(&text, "palettegen") && has_token(&text, "paletteuse");
        }

        // `aresample` exists in every build; the SoX resampler only when FFmpeg was built with it.
        let soxr_test = [
            "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "sine=duration=0.1",
            "-af", "aresample=resampler=soxr", "-f", "null", "-",
        ]
        .map(OsString::from);
        caps.has_soxr = matches!(
            run_hidden_output(&ff, &soxr_test, ProcessPriority::BelowNormal).await,
            Ok((0, _, _))
        );

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
                // Apple's own encoder on macOS (Apple Silicon and Intel).
                "h264_videotoolbox",
                "hevc_videotoolbox",
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
                .map_err(|e| tr!("Falha ao executar yt-dlp -v: {e}", "Couldn't run yt-dlp -v: {e}", "No se pudo ejecutar yt-dlp -v: {e}"))?;
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
                .map_err(|e| tr!("Falha listando extratores: {e}", "Couldn't list the extractors: {e}", "No se pudieron listar los extractores: {e}"))?;
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
            .map_err(|e| tr!("Sem conexão para checar atualização: {e}", "No connection to check for updates: {e}", "Sin conexión para buscar actualizaciones: {e}"))?
            .error_for_status()
            .map_err(|e| tr!("GitHub API respondeu com erro: {e}", "The GitHub API returned an error: {e}", "La API de GitHub respondió con un error: {e}"))?
            .json()
            .await
            .map_err(|e| tr!("Resposta inválida do GitHub: {e}", "Invalid response from GitHub: {e}", "Respuesta no válida de GitHub: {e}"))?;

        let cur = self.state.read().ytdlp_version.clone();
        if compare_versions(&rel.tag_name, &cur) > 0 {
            self.state.write().update_available = Some(rel.tag_name.clone());
            Ok(Some(rel.tag_name))
        } else {
            self.state.write().update_available = None;
            Ok(None)
        }
    }

    /// Updates `yt-dlp` with this system's release asset (`YTDLP_ASSET`), verifies `SHA2-256SUMS`,
    /// preserves previous version for rollback, and swaps atomically.
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
            .map_err(|e| tr!("Erro consultando GitHub: {e}", "Error querying GitHub: {e}", "Error al consultar GitHub: {e}"))?
            .error_for_status()
            .map_err(|e| tr!("Erro na API do GitHub: {e}", "GitHub API error: {e}", "Error en la API de GitHub: {e}"))?
            .json()
            .await
            .map_err(|e| tr!("JSON inválido do GitHub: {e}", "Invalid JSON from GitHub: {e}", "JSON no válido de GitHub: {e}"))?;

        let zip_asset = rel
            .assets
            .iter()
            .find(|a| a.name == YTDLP_ASSET)
            .ok_or_else(|| tr!("Asset {YTDLP_ASSET} não encontrado na release.", "{YTDLP_ASSET} asset not found in the release.", "No se encontró {YTDLP_ASSET} en la release."))?;
        let sums_asset = rel
            .assets
            .iter()
            .find(|a| a.name == "SHA2-256SUMS")
            .ok_or_else(|| tr!("Arquivo SHA2-256SUMS não encontrado na release.", "SHA2-256SUMS file not found in the release.", "No se encontró el archivo SHA2-256SUMS en la release."))?;

        let sums_text = client
            .get(&sums_asset.browser_download_url)
            .send()
            .await
            .map_err(|e| tr!("Falha baixando SHA2-256SUMS: {e}", "Couldn't download SHA2-256SUMS: {e}", "No se pudo descargar SHA2-256SUMS: {e}"))?
            .error_for_status()
            .map_err(|e| tr!("Erro HTTP em SHA2-256SUMS: {e}", "HTTP error on SHA2-256SUMS: {e}", "Error HTTP en SHA2-256SUMS: {e}"))?
            .text()
            .await
            .map_err(|e| e.to_string())?;

        let expected_sha = sums_text
            .lines()
            .find_map(|line| {
                let mut parts = line.split_whitespace();
                let hash = parts.next()?;
                let fname = parts.next()?.trim_start_matches('*');
                if fname == YTDLP_ASSET {
                    Some(hash.to_ascii_lowercase())
                } else {
                    None
                }
            })
            .ok_or_else(|| tr!("Hash do {YTDLP_ASSET} ausente em SHA2-256SUMS.", "{YTDLP_ASSET} hash missing from SHA2-256SUMS.", "Falta el hash de {YTDLP_ASSET} en SHA2-256SUMS."))?;

        let zip_bytes = client
            .get(&zip_asset.browser_download_url)
            .send()
            .await
            .map_err(|e| tr!("Falha baixando {YTDLP_ASSET}: {e}", "Couldn't download {YTDLP_ASSET}: {e}", "No se pudo descargar {YTDLP_ASSET}: {e}"))?
            .error_for_status()
            .map_err(|e| tr!("Erro HTTP baixando {YTDLP_ASSET}: {e}", "HTTP error downloading {YTDLP_ASSET}: {e}", "Error HTTP al descargar {YTDLP_ASSET}: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("Download interrompido: {e}"))?;

        let mut hasher = Sha256::new();
        hasher.update(&zip_bytes);
        let actual_sha = hex::encode(hasher.finalize());
        if actual_sha != expected_sha {
            return Err(tr!(
                "Verificação SHA-256 falhou! Esperado {expected_sha}, recebido {actual_sha}.",
                "SHA-256 check failed! Expected {expected_sha}, got {actual_sha}.",
                "¡Falló la verificación SHA-256! Se esperaba {expected_sha} y se recibió {actual_sha}."
            ));
        }

        let new_ver = rel.tag_name.clone();
        // The tag names a folder: only plain version characters, never a path.
        if new_ver.is_empty()
            || new_ver.starts_with('.')
            || !new_ver.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
        {
            return Err(tr!(
                "Versão inválida na release do yt-dlp.",
                "Invalid version in the yt-dlp release.",
                "Versión no válida en la release de yt-dlp."
            ));
        }
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
            // macOS: a single executable file.
            if !YTDLP_ASSET.ends_with(".zip") {
                let exe = tmp_clone.join(YTDLP_EXE);
                fs::write(&exe, &zip_vec).map_err(|e| e.to_string())?;
                set_unix_mode(&exe, 0o755)?;
                if dest_clone.exists() {
                    let _ = fs::remove_dir_all(&dest_clone);
                }
                return fs::rename(&tmp_clone, &dest_clone).map_err(|e| e.to_string());
            }
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
                    if let Some(mode) = file.unix_mode() {
                        set_unix_mode(&out_path, mode)?;
                    }
                }
            }

            // Locate folder containing the executable in case zip had a wrapper subdir
            let actual_root = find_file_parent(&tmp_clone, YTDLP_IN_ZIP)
                .ok_or_else(|| tr!("Executável do yt-dlp ausente no zip baixado.", "The yt-dlp executable is missing from the downloaded zip.", "Falta el ejecutable de yt-dlp en el zip descargado."))?;
            if YTDLP_IN_ZIP != YTDLP_EXE {
                fs::rename(actual_root.join(YTDLP_IN_ZIP), actual_root.join(YTDLP_EXE)).map_err(|e| e.to_string())?;
            }
            set_unix_mode(&actual_root.join(YTDLP_EXE), 0o755)?;

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
            .map_err(|_| tr!("Nenhuma versão anterior salva para restaurar.", "No previous version saved to restore.", "No hay una versión anterior guardada para restaurar."))?;
        let prev_ver = prev_ver.trim().to_string();
        let prev_exe = ytdlp_base
            .join("versions")
            .join(&prev_ver)
            .join(YTDLP_EXE);
        if !prev_exe.is_file() {
            return Err(tr!(
                "Binário da versão anterior ({prev_ver}) não está mais no disco.",
                "The previous version binary ({prev_ver}) is no longer on disk.",
                "El binario de la versión anterior ({prev_ver}) ya no está en el disco."
            ));
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
        .ok_or_else(|| tr!("Caminho inválido para extração", "Invalid extraction path", "Ruta de extracción no válida"))?;
    fs::create_dir_all(parent).map_err(|e| tr!("Erro criando pasta pai: {e}", "Error creating the parent folder: {e}", "Error al crear la carpeta superior: {e}"))?;

    let sibling_tmp = parent.join(format!(
        "{}.extracting-{}",
        dest_dir.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    if sibling_tmp.exists() {
        let _ = fs::remove_dir_all(&sibling_tmp);
    }
    fs::create_dir_all(&sibling_tmp).map_err(|e| tr!("Erro criando pasta temp: {e}", "Error creating the temp folder: {e}", "Error al crear la carpeta temporal: {e}"))?;

    {
        let mut st = mgr.state.write();
        st.stage = stage_label.to_string();
        st.component = Some(meta.key.to_string());
    }
    mgr.emit_state(app);

    let decoder = zstd::stream::read::Decoder::new(BufReader::new(compressed))
        .map_err(|e| {
                let key = &meta.key;
                tr!("Erro abrindo zstd {key}: {e}", "Error opening zstd {key}: {e}", "Error al abrir zstd {key}: {e}")
            })?;
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| {
                let key = &meta.key;
                tr!("Erro lendo tar {key}: {e}", "Error reading tar {key}: {e}", "Error al leer tar {key}: {e}")
            })?;

    let mut last_emit_bytes = *extracted_so_far;
    for entry_res in entries {
        let mut entry = entry_res.map_err(|e| format!("Entrada tar corrompida: {e}"))?;
        let rel_path = entry
            .path()
            .map_err(|e| tr!("Caminho tar inválido: {e}", "Invalid tar path: {e}", "Ruta tar no válida: {e}"))?
            .into_owned();
        let out_path = sibling_tmp.join(&rel_path);
        if let Some(p) = out_path.parent() {
            fs::create_dir_all(p).map_err(|e| tr!("Erro criando diretório: {e}", "Error creating the directory: {e}", "Error al crear el directorio: {e}"))?;
        }
        let mut out_file =
            File::create(&out_path).map_err(|e| {
                let path = out_path.display();
                tr!("Erro criando {path}: {e}", "Error creating {path}: {e}", "Error al crear {path}: {e}")
            })?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry
                .read(&mut buf)
                .map_err(|e| {
                let key = &meta.key;
                tr!("Erro descomprimindo {key}: {e}", "Error decompressing {key}: {e}", "Error al descomprimir {key}: {e}")
            })?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut out_file, &buf[..n])
                .map_err(|e| tr!("Erro gravando em disco: {e}", "Error writing to disk: {e}", "Error al escribir en el disco: {e}"))?;
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
        // Linux and macOS: the executables keep the permission they were packed with.
        if cfg!(unix) {
            if let Ok(mode) = entry.header().mode() {
                set_unix_mode(&out_path, mode)?;
            }
        }
    }

    let main_exe_path = sibling_tmp.join(main_exe_name);
    let main_exe_bytes = fs::metadata(&main_exe_path)
        .map_err(|e| tr!("Executável principal {main_exe_name} ausente após extração: {e}", "Main executable {main_exe_name} missing after extraction: {e}", "Falta el ejecutable principal {main_exe_name} después de la extracción: {e}"))?
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
        .map_err(|e| tr!("Erro gravando manifest: {e}", "Error writing the manifest: {e}", "Error al escribir el manifiesto: {e}"))?;

    if dest_dir.exists() {
        let backup = parent.join(format!(
            "{}.old-{}",
            dest_dir.file_name().unwrap().to_string_lossy(),
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&backup);
        fs::rename(dest_dir, &backup)
            .map_err(|e| {
                let dir = dest_dir.display();
                tr!(
                    "Binário em uso, não foi possível substituir {dir}: {e}",
                    "Binary in use, couldn't replace {dir}: {e}",
                    "Binario en uso, no se pudo reemplazar {dir}: {e}"
                )
            })?;
        fs::rename(&sibling_tmp, dest_dir)
            .map_err(|e| tr!("Erro movendo pasta extraída: {e}", "Error moving the extracted folder: {e}", "Error al mover la carpeta extraída: {e}"))?;
        let _ = fs::remove_dir_all(&backup);
    } else {
        fs::rename(&sibling_tmp, dest_dir)
            .map_err(|e| tr!("Erro ativando pasta extraída: {e}", "Error activating the extracted folder: {e}", "Error al activar la carpeta extraída: {e}"))?;
    }

    Ok(())
}

fn write_atomic_text(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, content).map_err(|e| {
        let path = tmp.display();
        tr!("Erro escrevendo {path}: {e}", "Error writing {path}: {e}", "Error al escribir {path}: {e}")
    })?;
    // `fs::rename` replaces the old file in one step; deleting it first would open a window in
    // which a crash leaves no file at all.
    fs::rename(&tmp, path).map_err(|e| {
        let path = path.display();
        tr!("Erro movendo {path}: {e}", "Error moving {path}: {e}", "Error al mover {path}: {e}")
    })
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

/// Applies Unix permissions (the executable bit) on Linux and macOS; nothing to do on Windows.
fn set_unix_mode(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777))
            .map_err(|e| format!("{}: {e}", path.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}
