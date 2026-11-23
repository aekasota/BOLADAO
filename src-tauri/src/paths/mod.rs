use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "Boladão";
pub const APP_SLUG: &str = "Boladao";
pub const TEMP_SUBDIR_NAME: &str = ".boladao-tmp";

/// Name of the app's own folders: `Boladao` on Windows and macOS, `boladao` on Linux (XDG habit).
#[cfg(target_os = "linux")]
const DATA_DIR_NAME: &str = "boladao";
#[cfg(not(target_os = "linux"))]
const DATA_DIR_NAME: &str = APP_SLUG;

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

/// settings.json and boladao.db: `%APPDATA%\Boladao`, `~/Library/Application Support/Boladao`
/// on macOS, `~/.config/boladao` on Linux.
pub fn app_config_dir() -> PathBuf {
    #[cfg(windows)]
    let base = env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Roaming")))
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(not(windows))]
    let base = dirs::config_dir().unwrap_or_else(|| user_home().join(".config"));
    let dir = base.join(DATA_DIR_NAME);
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Extracted engines, cookies, Whisper models and the media cache: `%LOCALAPPDATA%\Boladao`,
/// `~/Library/Application Support/Boladao` on macOS, `~/.local/share/boladao` on Linux.
pub fn app_local_dir() -> PathBuf {
    #[cfg(windows)]
    let base = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Local")))
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(not(windows))]
    let base = dirs::data_local_dir().unwrap_or_else(|| user_home().join(".local").join("share"));
    let dir = base.join(DATA_DIR_NAME);
    let _ = fs::create_dir_all(&dir);
    dir
}

/// The user's home folder.
pub fn user_home() -> PathBuf {
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE").map(PathBuf::from);
    #[cfg(not(windows))]
    let home = dirs::home_dir();
    home.unwrap_or_else(|| PathBuf::from("."))
}

/// Address of a file served by the app's own media protocol (thumbnails, waveforms,
/// spectrograms). WebView2 reaches custom protocols through `http://<name>.localhost`; WebKit on
/// Linux and macOS uses the scheme itself.
pub fn local_media_url(kind: &str, file_name: &str) -> String {
    #[cfg(windows)]
    let origin = "http://boladaomedia.localhost";
    #[cfg(not(windows))]
    let origin = "boladaomedia://localhost";
    format!("{origin}/{kind}/{file_name}")
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

/// The system's Music folder (localized on Linux, through the XDG user folders) plus `Boladão`.
pub fn default_music_root() -> PathBuf {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::FOLDERID_Music;
        if let Some(music) = known_folder_path(&FOLDERID_Music) {
            return music.join(APP_NAME);
        }
    }
    #[cfg(not(windows))]
    if let Some(music) = dirs::audio_dir() {
        return music.join(APP_NAME);
    }
    user_home().join("Music").join(APP_NAME)
}

/// The system's Videos folder (Movies on macOS) plus `Boladão`.
pub fn default_video_root() -> PathBuf {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::FOLDERID_Videos;
        if let Some(videos) = known_folder_path(&FOLDERID_Videos) {
            return videos.join(APP_NAME);
        }
    }
    #[cfg(not(windows))]
    if let Some(videos) = dirs::video_dir() {
        return videos.join(APP_NAME);
    }
    let folder = if cfg!(target_os = "macos") { "Movies" } else { "Videos" };
    user_home().join(folder).join(APP_NAME)
}

pub fn desktop_dir() -> PathBuf {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::FOLDERID_Desktop;
        if let Some(desktop) = known_folder_path(&FOLDERID_Desktop) {
            return desktop;
        }
    }
    #[cfg(not(windows))]
    if let Some(desktop) = dirs::desktop_dir() {
        return desktop;
    }
    user_home().join("Desktop")
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

/// Free bytes the user can write on the volume holding `target_dir` (`GetDiskFreeSpaceExW` on
/// Windows, `statvfs` elsewhere).
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

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(probe.as_os_str().as_bytes()).ok()?;
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: `path` is a valid C string and `statvfs` fills `stats` when it returns 0.
        if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: initialized by the successful call above.
        let stats = unsafe { stats.assume_init() };
        (stats.f_bavail as u64).checked_mul(stats.f_frsize as u64)
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

/// Normalizes separators (`\` and `/` both become the system's own) and rewrites English and
/// Spanish tokens to the canonical ones.
pub fn canonical_template(template: &str) -> String {
    let mut out = template.replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR);
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
    let out = out.trim_start_matches(std::path::MAIN_SEPARATOR).to_string();
    if out.ends_with(".%(ext)s") {
        out
    } else {
        format!("{out}.%(ext)s")
    }
}

/// Computes a safe `--trim-filenames` value considering root path depth and Windows MAX_PATH (259 chars).
/// Linux and macOS have no such path limit, but each name holds 255 bytes: 80 characters fit
/// even when every one takes 3 bytes (Japanese, Chinese, Korean).
pub fn compute_safe_trim_length(root_dir: &Path, template: &str) -> usize {
    if cfg!(not(windows)) {
        return 80;
    }
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
        .split(std::path::MAIN_SEPARATOR)
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

/// Moves every file a "Keep both" job left in its own folder into `root`, keeping the
/// subfolders, and returns each file's old and new path. When any of them would land on an
/// existing file, the whole set takes the first free ` (n)`, so a video and its subtitles keep
/// matching names (`clip (2).mp4`, `clip (2).pt.srt`).
pub fn deliver_keeping_both(
    staging: &Path,
    root: &Path,
    primary_stem: &str,
) -> std::io::Result<Vec<(PathBuf, PathBuf)>> {
    let mut staged = Vec::new();
    collect_files(staging, &mut staged)?;
    let targets: Vec<(PathBuf, PathBuf)> = staged
        .into_iter()
        .filter_map(|from| {
            let to = root.join(from.strip_prefix(staging).ok()?);
            Some((from, to))
        })
        .collect();

    let number = if targets.iter().any(|(_, to)| to.exists()) {
        let taken = |n: u32| {
            targets
                .iter()
                .any(|(_, to)| numbered(to, n, primary_stem).exists())
        };
        let mut n = 2;
        while taken(n) {
            n += 1;
        }
        Some(n)
    } else {
        None
    };

    let mut moved = Vec::with_capacity(targets.len());
    for (from, to) in targets {
        let to = match number {
            Some(n) => numbered(&to, n, primary_stem),
            None => to,
        };
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&from, &to)?;
        moved.push((from, to));
    }
    let _ = fs::remove_dir_all(staging);
    Ok(moved)
}

/// `clip.pt.srt` -> `clip (2).pt.srt` for files named after the main one (`clip`); any other file
/// takes the number before its extension (`01 - Intro.mp4` -> `01 - Intro (2).mp4`).
fn numbered(path: &Path, n: u32, primary_stem: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let renamed = match name.strip_prefix(primary_stem) {
        Some(rest) if !primary_stem.is_empty() && (rest.is_empty() || rest.starts_with('.')) => {
            format!("{primary_stem} ({n}){rest}")
        }
        _ => match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({n}).{ext}"),
            _ => format!("{name} ({n})"),
        },
    };
    path.with_file_name(renamed)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// Whether `path` was created before `moment` (false when it is missing or the disk keeps no
/// creation time).
pub fn created_before(path: &Path, moment: std::time::SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|m| m.created())
        .map(|created| created < moment)
        .unwrap_or(false)
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

/// Program that opens files and links with the user's default app: `open` on macOS,
/// `xdg-open` on Linux.
#[cfg(unix)]
const OPENER: &str = if cfg!(target_os = "macos") { "/usr/bin/open" } else { "xdg-open" };

/// Starts `program` without waiting for it; a thread reaps it so it never lingers as a zombie.
#[cfg(unix)]
fn spawn_detached(program: &str, args: &[&std::ffi::OsStr]) -> std::io::Result<()> {
    use std::process::Stdio;
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Opens a file with the default OS handler (`ShellExecute`, `open`, `xdg-open`).
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
    #[cfg(unix)]
    {
        // An absolute path starts with `/`, so `open` and `xdg-open` never take it for an option.
        std::path::absolute(path)
            .and_then(|path| spawn_detached(OPENER, &[path.as_os_str()]))
            .map_err(|e| tr!("Falha ao abrir arquivo: {e}", "Couldn't open the file: {e}", "No se pudo abrir el archivo: {e}"))
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
    #[cfg(unix)]
    {
        spawn_detached(OPENER, &[std::ffi::OsStr::new(url)])
            .map_err(|e| tr!("Falha ao abrir o navegador: {e}", "Couldn't open the browser: {e}", "No se pudo abrir el navegador: {e}"))
    }
}

/// Shows the file selected in the system's file manager: `explorer.exe /select,<path>` on
/// Windows, Finder (`open -R`) on macOS, and on Linux the file manager's `ShowItems` (GNOME
/// Files, Dolphin, Nemo...), falling back to opening the folder.
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
    #[cfg(unix)]
    {
        let fail = |e: std::io::Error| {
            tr!("Falha ao abrir pasta: {e}", "Couldn't open the folder: {e}", "No se pudo abrir la carpeta: {e}")
        };
        // An absolute path starts with `/`, so `open` and `xdg-open` never take it for an option.
        let absolute = std::path::absolute(path).map_err(&fail)?;
        let path = absolute.as_path();
        if !path.exists() {
            let parent = path.parent().filter(|p| p.exists()).ok_or_else(|| {
                tr!("Pasta ou arquivo não encontrado.", "Folder or file not found.", "Carpeta o archivo no encontrado.")
            })?;
            return spawn_detached(OPENER, &[parent.as_os_str()]).map_err(fail);
        }
        if path.is_dir() {
            return spawn_detached(OPENER, &[path.as_os_str()]).map_err(fail);
        }
        #[cfg(target_os = "macos")]
        {
            spawn_detached(OPENER, &[std::ffi::OsStr::new("-R"), path.as_os_str()]).map_err(fail)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let shown = url::Url::from_file_path(path).ok().is_some_and(|uri| {
                std::process::Command::new("dbus-send")
                    .args([
                        "--session",
                        "--print-reply",
                        "--dest=org.freedesktop.FileManager1",
                        "--type=method_call",
                        "/org/freedesktop/FileManager1",
                        "org.freedesktop.FileManager1.ShowItems",
                    ])
                    // dbus-send splits array items on commas.
                    .arg(format!("array:string:{}", uri.as_str().replace(',', "%2C")))
                    .arg("string:")
                    .output()
                    .is_ok_and(|out| out.status.success())
            });
            if shown {
                return Ok(());
            }
            let folder = path.parent().unwrap_or(path);
            spawn_detached(OPENER, &[folder.as_os_str()]).map_err(fail)
        }
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
        // Templates written with either slash compile with the system's own separator.
        let sep = std::path::MAIN_SEPARATOR_STR;
        let audio = compile_ytdlp_output_template("{artista}\\{album}\\{faixa}");
        assert_eq!(
            audio,
            "%(artist,creator,uploader,channel|Artista)s\\%(album,playlist_title|Singles)s\\%(track,title|Faixa)s.%(ext)s"
                .replace('\\', sep)
        );

        let video = compile_ytdlp_output_template("{plataforma}/{canal}/{titulo} [{id}]");
        assert_eq!(
            video,
            "%(extractor_key|Web)s\\%(channel,uploader,creator,artist|Canal)s\\%(title| Sem título)s [%(id)s].%(ext)s"
                .replace('\\', sep)
        );
    }

    #[cfg(unix)]
    #[test]
    fn previews_keep_every_name_within_255_bytes() {
        let sample = PathPreviewSample {
            title: "長".repeat(200),
            creator: "CON".to_string(),
            ext: "mp4".to_string(),
            ..Default::default()
        };
        let preview = preview_output_path(Path::new("/home/ana/Vídeos/Boladão"), "{canal}\\{titulo}", &sample);
        assert_eq!(preview.parent().unwrap().file_name().unwrap(), "_CON");
        let name = preview.file_name().unwrap().to_string_lossy();
        assert!(name.len() <= 255, "nome com {} bytes", name.len());
    }

    #[cfg(windows)]
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

    #[test]
    fn keep_both_numbers_the_whole_new_download_and_leaves_the_old_files_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Música");
        let staging = root.join(TEMP_SUBDIR_NAME).join("1a2b3c4d");
        let dir = root.join("Artista");
        let staged_dir = staging.join("Artista");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&staged_dir).unwrap();
        fs::write(dir.join("Faixa.mp3"), "old").unwrap();
        // An older second copy left its subtitles, so " (2)" is taken for this set.
        fs::write(dir.join("Faixa (2).pt.srt"), "old subs").unwrap();
        fs::write(staged_dir.join("Faixa.mp3"), "new").unwrap();
        fs::write(staged_dir.join("Faixa.pt.srt"), "new subs").unwrap();

        let moved = deliver_keeping_both(&staging, &root, "Faixa").unwrap();

        assert_eq!(fs::read_to_string(dir.join("Faixa.mp3")).unwrap(), "old");
        assert_eq!(fs::read_to_string(dir.join("Faixa (2).pt.srt")).unwrap(), "old subs");
        assert_eq!(fs::read_to_string(dir.join("Faixa (3).mp3")).unwrap(), "new");
        // The subtitles take the same number, so players still pair them with the audio.
        assert_eq!(fs::read_to_string(dir.join("Faixa (3).pt.srt")).unwrap(), "new subs");
        assert!(moved.contains(&(staged_dir.join("Faixa.mp3"), dir.join("Faixa (3).mp3"))));
        assert!(!staging.exists());
    }

    #[test]
    fn keep_both_keeps_the_names_when_nothing_is_in_the_way() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let staging = root.join(TEMP_SUBDIR_NAME).join("1a2b3c4d");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("clip.mp4"), "video").unwrap();
        fs::write(staging.join("01 - Intro.mp4"), "chapter").unwrap();
        fs::write(root.join("clip (2).mp4"), "unrelated").unwrap();

        let moved = deliver_keeping_both(&staging, &root, "clip").unwrap();

        assert_eq!(moved.len(), 2);
        assert_eq!(fs::read_to_string(root.join("clip.mp4")).unwrap(), "video");
        assert_eq!(fs::read_to_string(root.join("01 - Intro.mp4")).unwrap(), "chapter");
        assert_eq!(fs::read_to_string(root.join("clip (2).mp4")).unwrap(), "unrelated");
    }

    #[test]
    fn tells_files_created_before_a_moment() {
        use std::time::{Duration, SystemTime};
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.mp3");
        fs::write(&file, "x").unwrap();
        let now = SystemTime::now();

        assert!(created_before(&file, now + Duration::from_secs(60)));
        assert!(!created_before(&file, now - Duration::from_secs(3600)));
        assert!(!created_before(&tmp.path().join("missing.mp3"), now + Duration::from_secs(60)));
    }
}
