use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "Boladão";
pub const APP_SLUG: &str = "Boladao";
pub const TEMP_SUBDIR_NAME: &str = ".boladao-tmp";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollisionPolicy {
    KeepBoth,
    Overwrite,
    Skip,
}

impl Default for CollisionPolicy {
    fn default() -> Self {
        Self::KeepBoth
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TemplateToken {
    MediaType,
    Platform,
    Creator,
    PlaylistOrAlbum,
    PlaylistIndex,
    Title,
    TrackOrTitle,
    Id,
    Year,
    Month,
    Literal(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformPathRule {
    pub platform: String,
    pub enabled: bool,
    pub custom_root: Option<PathBuf>,
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathPreviewSample {
    pub media_type: String,
    pub platform: String,
    pub creator: String,
    pub playlist: Option<String>,
    pub index: Option<u32>,
    pub title: String,
    pub track: Option<String>,
    pub id: String,
    pub year: String,
    pub month: String,
    pub ext: String,
}

impl Default for PathPreviewSample {
    fn default() -> Self {
        Self {
            media_type: tr!("Vídeos", "Videos", "Videos"),
            platform: "YouTube".to_string(),
            creator: tr!("Canal Exemplo", "Example Channel", "Canal de ejemplo"),
            playlist: Some(tr!("Favoritos", "Favorites", "Favoritos")),
            index: Some(1),
            title: tr!("Vídeo de Teste", "Test Video", "Video de prueba"),
            track: Some(tr!("Faixa de Teste", "Test Track", "Pista de prueba")),
            id: "dQw4w9WgXcQ".to_string(),
            year: "2026".to_string(),
            month: "11".to_string(),
            ext: "mp4".to_string(),
        }
    }
}

/// `%APPDATA%\Boladao` — settings.json and boladao.db
pub fn app_config_dir() -> PathBuf {
    let base = env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Roaming")))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join(APP_SLUG);
    let _ = fs::create_dir_all(&dir);
    dir
}

/// `%LOCALAPPDATA%\Boladao` — extracted binaries, cookies, Whisper models, media cache
pub fn app_local_dir() -> PathBuf {
    let base = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Local")))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join(APP_SLUG);
    let _ = fs::create_dir_all(&dir);
    dir
}

#[cfg(windows)]
fn known_folder_path(rfid: &windows_sys::core::GUID) -> Option<PathBuf> {
    unsafe {
        use windows_sys::Win32::UI::Shell::SHGetKnownFolderPath;
        let mut raw_ptr: *mut u16 = std::ptr::null_mut();
        let hr = SHGetKnownFolderPath(rfid, 0, std::ptr::null_mut(), &mut raw_ptr);
        if hr != 0 || raw_ptr.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *raw_ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(raw_ptr, len);
        let s = String::from_utf16_lossy(slice);
        // Free CoTaskMem
        extern "system" {
            fn CoTaskMemFree(pv: *const std::ffi::c_void);
        }
        CoTaskMemFree(raw_ptr as *const _);
        Some(PathBuf::from(s))
    }
}

pub fn default_music_root() -> PathBuf {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::FOLDERID_Music;
        if let Some(music) = known_folder_path(&FOLDERID_Music) {
            return music.join(APP_NAME);
        }
    }
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Music")
        .join(APP_NAME)
}

pub fn default_video_root() -> PathBuf {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::FOLDERID_Videos;
        if let Some(videos) = known_folder_path(&FOLDERID_Videos) {
            return videos.join(APP_NAME);
        }
    }
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Videos")
        .join(APP_NAME)
}

pub fn desktop_dir() -> PathBuf {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::FOLDERID_Desktop;
        if let Some(desktop) = known_folder_path(&FOLDERID_Desktop) {
            return desktop;
        }
    }
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Desktop")
}

pub fn start_menu_programs_dir() -> PathBuf {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::FOLDERID_Programs;
        if let Some(progs) = known_folder_path(&FOLDERID_Programs) {
            return progs;
        }
    }
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
}

pub fn default_install_dir() -> PathBuf {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::FOLDERID_LocalAppData;
        if let Some(local) = known_folder_path(&FOLDERID_LocalAppData) {
            return local.join("Programs").join(APP_NAME);
        }
    }
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Programs")
        .join(APP_NAME)
}

/// Checks for legacy `yt2` `config.json` next to the `.exe` or in `%USERPROFILE%\Documents\h\yt2\config.json`.
pub fn detect_legacy_yt2_music_folder() -> Option<(PathBuf, PathBuf)> {
    let mut candidates = Vec::new();
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("config.json"));
        }
    }
    if let Some(profile) = env::var_os("USERPROFILE") {
        candidates.push(
            PathBuf::from(profile)
                .join("Documents")
                .join("h")
                .join("yt2")
                .join("config.json"),
        );
    }

    #[derive(Deserialize)]
    struct LegacyYt2Config {
        destination_folder: Option<String>,
    }

    for cfg_path in candidates {
        if cfg_path.is_file() {
            if let Ok(raw) = fs::read_to_string(&cfg_path) {
                let clean_raw = raw.trim_start_matches('\u{feff}');
                if let Ok(parsed) = serde_json::from_str::<LegacyYt2Config>(clean_raw) {
                    if let Some(dest) = parsed.destination_folder {
                        let trimmed = dest.trim();
                        if !trimmed.is_empty() {
                            return Some((PathBuf::from(trimmed), cfg_path));
                        }
                    }
                }
            }
        }
    }
    None
}

/// Queries available free bytes on the volume containing `target_dir` via `GetDiskFreeSpaceExW`.
pub fn free_disk_bytes(target_dir: &Path) -> Option<u64> {
    // Walk up until we find an existing ancestor or drive root
    let mut probe = target_dir.to_path_buf();
    while !probe.exists() {
        if let Some(parent) = probe.parent() {
            if parent == probe {
                break;
            }
            probe = parent.to_path_buf();
        } else {
            break;
        }
    }
    if !probe.exists() {
        probe = env::current_dir().ok()?;
    }

    #[cfg(windows)]
    unsafe {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

        let wide: Vec<u16> = probe
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut free_available: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut total_free: u64 = 0;
        let ok = GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_available,
            &mut total_bytes,
            &mut total_free,
        );
        if ok != 0 {
            return Some(free_available);
        }
        None
    }

    #[cfg(not(windows))]
    {
        let _ = probe;
        None
    }
}

pub fn extract_drive_label(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.len() >= 2 && s.as_bytes()[1] == b':' {
        s[..2].to_uppercase()
    } else {
        path.display().to_string()
    }
}

/// Sanitizes a single Windows path segment (removes forbidden characters, trailing dots/spaces,
/// and prefixes reserved DOS device names such as `CON`, `NUL`, `COM1`, `LPT1`).
pub fn sanitize_windows_component(raw: &str) -> String {
    let mut cleaned = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let invalid = matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            || (ch as u32) < 32;
        if invalid {
            cleaned.push('_');
        } else {
            cleaned.push(ch);
        }
    }
    let trimmed = cleaned.trim_matches(|c: char| c == ' ' || c == '.');
    let candidate = if trimmed.is_empty() { "_" } else { trimmed };

    let stem = candidate
        .split('.')
        .next()
        .unwrap_or(candidate)
        .trim()
        .to_ascii_uppercase();

    let is_reserved = matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );

    if is_reserved {
        format!("_{candidate}")
    } else {
        candidate.to_string()
    }
}

/// Converts a user-friendly template string with `{placeholders}` into a valid `yt-dlp` `--output` template
/// with robust field fallbacks (`%(artist,creator,uploader,channel|Desconhecido)s`, etc.).
///
/// Supported placeholders:
/// Template tokens in English and Spanish, mapped to the original Portuguese ones, so a template
/// keeps working whatever language it was written in.
const TOKEN_ALIASES: &[(&str, &str)] = &[
    ("{type}", "{tipo}"),
    ("{platform}", "{plataforma}"),
    ("{channel}", "{canal}"),
    ("{artist}", "{artista}"),
    ("{álbum}", "{album}"),
    ("{title}", "{titulo}"),
    ("{título}", "{titulo}"),
    ("{track}", "{faixa}"),
    ("{pista}", "{faixa}"),
    ("{year}", "{ano}"),
    ("{año}", "{ano}"),
    ("{month}", "{mes}"),
];

/// Normalizes separators and rewrites English and Spanish tokens to the canonical ones.
pub fn canonical_template(template: &str) -> String {
    let mut out = template.replace('/', "\\");
    for (alias, canonical) in TOKEN_ALIASES {
        out = out.replace(alias, canonical);
    }
    out
}

/// yt-dlp field for the title, with the "untitled" fallback of the interface language.
pub fn untitled_template() -> String {
    format!("%(title|{})s", tr!(" Sem título", " Untitled", " Sin título"))
}

/// `{tipo}`, `{plataforma}`, `{canal}`, `{artista}`, `{playlist}`, `{album}`,
/// `{nn}`, `{titulo}`, `{faixa}`, `{id}`, `{ano}`, `{mes}`
pub fn compile_ytdlp_output_template(template: &str) -> String {
    let normalized = canonical_template(template);
    // Fallbacks for metadata the site does not provide, in the interface language.
    let kind = format!("%(boladao_kind|{})s", tr!("Mídia", "Media", "Medios"));
    let channel = format!(
        "%(channel,uploader,creator,artist|{})s",
        tr!("Canal", "Channel", "Canal")
    );
    let artist = format!(
        "%(artist,creator,uploader,channel|{})s",
        tr!("Artista", "Artist", "Artista")
    );
    let loose = format!("%(playlist_title,album|{})s", tr!("Avulsos", "Loose", "Sueltos"));
    let title = untitled_template();
    let track = format!("%(track,title|{})s", tr!("Faixa", "Track", "Pista"));
    let replacements: &[(&str, &str)] = &[
        ("{tipo}", &kind),
        ("{plataforma}", "%(extractor_key|Web)s"),
        ("{canal}", &channel),
        ("{artista}", &artist),
        ("{playlist}", &loose),
        ("{album}", "%(album,playlist_title|Singles)s"),
        ("{nn}", "%(playlist_index|01)02d"),
        ("{titulo}", &title),
        ("{faixa}", &track),
        ("{id}", "%(id)s"),
        ("{ano}", "%(upload_date>%Y,release_year|2026)s"),
        ("{mes}", "%(upload_date>%m|01)s"),
    ];

    let mut out = normalized;
    for (token, ytdlp_expr) in replacements {
        out = out.replace(token, ytdlp_expr);
    }
    // Strip leading slashes so `--paths home:<root>` stays authoritative
    let out = out.trim_start_matches('\\').to_string();
    if out.ends_with(".%(ext)s") {
        out
    } else {
        format!("{out}.%(ext)s")
    }
}

/// Computes a safe `--trim-filenames` value considering root path depth and Windows MAX_PATH (259 chars).
pub fn compute_safe_trim_length(root_dir: &Path, template: &str) -> usize {
    let root_len = root_dir.to_string_lossy().chars().count();
    let depth = template.matches('\\').count().saturating_add(template.matches('/').count());
    // Leave comfortable headroom inside classic 259-char Win32 limit for `.part` / `.ytdl` / `(2)` suffixes
    let available = 235usize.saturating_sub(root_len + depth * 12);
    available.clamp(40, 180)
}

/// Renders a live human-readable preview of the final resolved path for the UI.
pub fn preview_output_path(root_dir: &Path, template: &str, sample: &PathPreviewSample) -> PathBuf {
    let playlist_val = sample
        .playlist
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Singles");
    let track_val = sample
        .track
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&sample.title);
    let idx_str = format!("{:02}", sample.index.unwrap_or(1));

    let mut map = HashMap::new();
    map.insert("{tipo}", sample.media_type.as_str());
    map.insert("{plataforma}", sample.platform.as_str());
    map.insert("{canal}", sample.creator.as_str());
    map.insert("{artista}", sample.creator.as_str());
    map.insert("{playlist}", playlist_val);
    map.insert("{album}", playlist_val);
    map.insert("{nn}", idx_str.as_str());
    map.insert("{titulo}", sample.title.as_str());
    map.insert("{faixa}", track_val);
    map.insert("{id}", sample.id.as_str());
    map.insert("{ano}", sample.ano_or_default());
    map.insert("{mes}", sample.mes_or_default());

    let mut rendered = canonical_template(template);
    for (k, v) in map {
        rendered = rendered.replace(k, v);
    }

    let trim_limit = compute_safe_trim_length(root_dir, template);
    let mut final_path = root_dir.to_path_buf();
    let segments: Vec<&str> = rendered
        .split('\\')
        .filter(|s| !s.trim().is_empty())
        .collect();

    for (i, seg) in segments.iter().enumerate() {
        let mut clean = sanitize_windows_component(seg);
        if clean.chars().count() > trim_limit {
            clean = clean.chars().take(trim_limit).collect();
        }
        if i + 1 == segments.len() {
            let ext = sample.ext.trim_start_matches('.');
            final_path.push(format!("{clean}.{ext}"));
        } else {
            final_path.push(clean);
        }
    }
    final_path
}

impl PathPreviewSample {
    fn ano_or_default(&self) -> &str {
        if self.year.is_empty() {
            "2026"
        } else {
            &self.year
        }
    }
    fn mes_or_default(&self) -> &str {
        if self.month.is_empty() {
            "01"
        } else {
            &self.month
        }
    }
}

/// Resolves filename collisions when `CollisionPolicy::KeepBoth` is active: `song.mp3` -> `song (2).mp3`.
pub fn next_available_collision_path(desired: &Path) -> PathBuf {
    if !desired.exists() {
        return desired.to_path_buf();
    }
    let parent = desired.parent().unwrap_or_else(|| Path::new("."));
    let stem = desired
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "arquivo".to_string());
    let ext = desired
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    for n in 2..=9999 {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem} ({}){ext}", uuid::Uuid::new_v4().simple()))
}

/// Cleans leftover `.boladao-tmp` folders inside configured library roots at startup (only when no active jobs own them).
/// Removes `<root>\.boladao-tmp` for each root. Callers must only pass folders where no job can still
/// resume from its partial files (see `QueueManager::clean_temp_if_idle`).
pub fn cleanup_stale_temp_dirs(roots: &[PathBuf]) {
    for root in roots {
        let tmp = root.join(TEMP_SUBDIR_NAME);
        if tmp.is_dir() {
            let _ = fs::remove_dir_all(&tmp);
        }
    }
}

/// Opens a file with the default OS handler (`ShellExecute` / `explorer`).
pub fn open_file_default(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(tr!("Arquivo não encontrado no disco.", "File not found on disk.", "Archivo no encontrado en el disco."));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32.exe")
            .arg("url.dll,FileProtocolHandler")
            .arg(path)
            .creation_flags(crate::winproc::CREATE_NO_WINDOW_FLAG)
            .spawn()
            .map_err(|e| tr!("Falha ao abrir arquivo: {e}", "Couldn't open the file: {e}", "No se pudo abrir el archivo: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("Apenas Windows suportado.".to_string())
    }
}

/// Opens an `https://` link in the default browser.
pub fn open_url_default(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(tr!("Link inválido.", "Invalid link.", "Enlace no válido."));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32.exe")
            .arg("url.dll,FileProtocolHandler")
            .arg(url)
            .creation_flags(crate::winproc::CREATE_NO_WINDOW_FLAG)
            .spawn()
            .map_err(|e| tr!("Falha ao abrir o navegador: {e}", "Couldn't open the browser: {e}", "No se pudo abrir el navegador: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("Apenas Windows suportado.".to_string())
    }
}

/// Opens Windows Explorer with the target file selected (`explorer.exe /select,<path>`).
pub fn reveal_in_explorer(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if path.exists() {
            let mut select_arg = std::ffi::OsString::from("/select,");
            select_arg.push(path.as_os_str());
            std::process::Command::new("explorer.exe")
                .arg(select_arg)
                .creation_flags(crate::winproc::CREATE_NO_WINDOW_FLAG)
                .spawn()
                .map_err(|e| tr!("Falha ao abrir pasta: {e}", "Couldn't open the folder: {e}", "No se pudo abrir la carpeta: {e}"))?;
        } else if let Some(parent) = path.parent().filter(|p| p.exists()) {
            std::process::Command::new("explorer.exe")
                .arg(parent)
                .creation_flags(crate::winproc::CREATE_NO_WINDOW_FLAG)
                .spawn()
                .map_err(|e| tr!("Falha ao abrir pasta: {e}", "Couldn't open the folder: {e}", "No se pudo abrir la carpeta: {e}"))?;
        } else {
            return Err(tr!("Pasta ou arquivo não encontrado.", "Folder or file not found.", "Carpeta o archivo no encontrado."));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("Apenas Windows suportado.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_windows_reserved_and_illegal_names() {
        assert_eq!(sanitize_windows_component("CON"), "_CON");
        assert_eq!(sanitize_windows_component("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_windows_component("lpt1"), "_lpt1");
        assert_eq!(
            sanitize_windows_component("AC/DC: Live? *2026*... "),
            "AC_DC_ Live_ _2026_"
        );
        assert_eq!(sanitize_windows_component("...   "), "_");
    }

    #[test]
    fn compiles_templates_to_ytdlp_output_spec() {
        let audio = compile_ytdlp_output_template("{artista}\\{album}\\{faixa}");
        assert_eq!(
            audio,
            "%(artist,creator,uploader,channel|Artista)s\\%(album,playlist_title|Singles)s\\%(track,title|Faixa)s.%(ext)s"
        );

        let video = compile_ytdlp_output_template("{plataforma}/{canal}/{titulo} [{id}]");
        assert_eq!(
            video,
            "%(extractor_key|Web)s\\%(channel,uploader,creator,artist|Canal)s\\%(title| Sem título)s [%(id)s].%(ext)s"
        );
    }

    #[test]
    fn previews_and_trims_deep_windows_paths() {
        let deep_root = PathBuf::from(
            r"C:\Users\leonardo\Documents\BibliotecaGiganteDeMidiasPessoais\Colecao2026\SubpastaExtraLonga",
        );
        let sample = PathPreviewSample {
            title: "A".repeat(240),
            creator: "CON".to_string(),
            ext: "mp4".to_string(),
            ..Default::default()
        };
        let preview = preview_output_path(&deep_root, "{canal}\\{titulo}", &sample);
        let full = preview.to_string_lossy();
        assert!(full.contains("\\_CON\\"));
        assert!(
            full.chars().count() < 260,
            "Path exceeded MAX_PATH: len={}",
            full.chars().count()
        );
    }
}
