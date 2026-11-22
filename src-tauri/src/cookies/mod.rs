use crate::args::CookieSource;
use crate::paths::app_local_dir;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountAuthKind {
    EmbeddedCookies,
    ImportedCookiesFile,
    FirefoxBrowser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedAccount {
    pub platform_id: String,
    pub auth_kind: AccountAuthKind,
    pub connected_at: DateTime<Utc>,
    pub earliest_expiry_ts: Option<i64>,
    pub cookie_count: usize,
}

#[derive(Debug, Clone)]
pub struct NetscapeCookieEntry {
    pub domain: String,
    pub include_subdomains: bool,
    pub path: String,
    pub secure: bool,
    pub expires_unix: u64,
    pub http_only: bool,
    pub name: String,
    pub value: String,
}

pub fn cookies_dir() -> PathBuf {
    let dir = app_local_dir().join("cookies");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn webview_profiles_dir() -> PathBuf {
    let dir = app_local_dir().join("webview-profiles");
    let _ = fs::create_dir_all(&dir);
    dir
}

// Session cookies work like passwords. They are stored as `<platform>.cookies`, encrypted with the
// Windows Data Protection API (DPAPI, bound to the current Windows user), so a copied folder or a
// backup of it is useless elsewhere. yt-dlp only reads plaintext Netscape files, so a copy lives in
// `cookies\run\` strictly while a yt-dlp process runs (see `CookieLease`).
const DPAPI_ENTROPY: &[u8] = b"Boladao/cookies/v1";

fn account_key(platform_id: &str) -> &str {
    if platform_id == "youtube_music" {
        "youtube"
    } else {
        platform_id
    }
}

fn store_file(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.cookies"))
}

/// Plaintext file written by v1.0.0, migrated to the encrypted store on startup or first use.
fn legacy_file(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.txt"))
}

fn run_dir(dir: &Path) -> PathBuf {
    dir.join("run")
}

fn save_store_in(dir: &Path, key: &str, plaintext: &str) -> Result<(), String> {
    let blob = dpapi_protect(plaintext.as_bytes())?;
    fs::create_dir_all(dir).map_err(|e| tr!("Erro criando a pasta de cookies: {e}", "Error creating the cookies folder: {e}", "Error al crear la carpeta de cookies: {e}"))?;
    let tmp = dir.join(format!("{key}.{}.tmp", Uuid::new_v4().simple()));
    fs::write(&tmp, blob).map_err(|e| tr!("Erro salvando cookies: {e}", "Error saving cookies: {e}", "Error al guardar las cookies: {e}"))?;
    fs::rename(&tmp, store_file(dir, key)).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        tr!("Erro salvando cookies: {e}", "Error saving cookies: {e}", "Error al guardar las cookies: {e}")
    })?;
    let _ = fs::remove_file(legacy_file(dir, key));
    Ok(())
}

fn load_store_in(dir: &Path, key: &str) -> Option<String> {
    if let Ok(blob) = fs::read(store_file(dir, key)) {
        return dpapi_unprotect(&blob).ok().and_then(|plain| String::from_utf8(plain).ok());
    }
    let legacy = fs::read_to_string(legacy_file(dir, key)).ok()?;
    let _ = save_store_in(dir, key, &legacy);
    Some(legacy)
}

fn migrate_legacy_in(dir: &Path) {
    // Plaintext copies only exist while yt-dlp runs; leftovers mean the app was killed mid-run.
    let _ = fs::remove_dir_all(run_dir(dir));
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|ext| ext == "txt").unwrap_or(false) {
            let key = path.file_stem().and_then(|s| s.to_str()).map(str::to_string);
            if let (Some(key), Ok(raw)) = (key, fs::read_to_string(&path)) {
                if save_store_in(dir, &key, &raw).is_ok() {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }
}

/// Encrypts cookie files left in plaintext by v1.0.0 and removes stray plaintext copies.
pub fn secure_cookie_store() {
    migrate_legacy_in(&cookies_dir());
}

/// Plaintext cookie file handed to yt-dlp for a single run. Keep it alive until the process exits:
/// dropping it encrypts the cookies yt-dlp refreshed back into the store and deletes the copy.
pub struct CookieLease {
    dir: PathBuf,
    key: String,
    path: PathBuf,
    original: String,
}

impl CookieLease {
    fn open_in(dir: &Path, key: &str) -> Option<Self> {
        let plaintext = load_store_in(dir, key)?;
        let run = run_dir(dir);
        fs::create_dir_all(&run).ok()?;
        let path = run.join(format!("{key}-{}.txt", Uuid::new_v4().simple()));
        fs::write(&path, &plaintext).ok()?;
        Some(Self {
            dir: dir.to_path_buf(),
            key: key.to_string(),
            path,
            original: plaintext,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CookieLease {
    fn drop(&mut self) {
        // yt-dlp dumps its cookie jar back into this file on exit. A run killed mid-write can leave
        // a truncated file, so only a file that still holds cookies replaces the stored ones.
        if let Ok(updated) = fs::read_to_string(&self.path) {
            if updated != self.original && netscape_cookie_stats(&updated).0 > 0 {
                let _ = save_store_in(&self.dir, &self.key, &updated);
            }
        }
        let _ = fs::remove_file(&self.path);
    }
}

/// Decides how yt-dlp receives cookies for a platform. When a file is needed, the returned lease
/// must outlive the yt-dlp process that reads it.
pub fn prepare_cookie_source(
    platform_id: &str,
    accounts: &BTreeMap<String, ConnectedAccount>,
) -> (CookieSource, Option<CookieLease>) {
    let key = account_key(platform_id);
    match accounts.get(key).map(|acc| acc.auth_kind) {
        Some(AccountAuthKind::FirefoxBrowser) => (CookieSource::FirefoxBrowser, None),
        Some(AccountAuthKind::EmbeddedCookies | AccountAuthKind::ImportedCookiesFile) => {
            match CookieLease::open_in(&cookies_dir(), key) {
                Some(lease) => (CookieSource::File(lease.path().to_path_buf()), Some(lease)),
                None => (CookieSource::None, None),
            }
        }
        None => (CookieSource::None, None),
    }
}

/// Counts valid Netscape cookie lines and finds the earliest expiry among them.
pub fn netscape_cookie_stats(raw: &str) -> (usize, Option<i64>) {
    let mut count = 0usize;
    let mut earliest: Option<i64> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || (trimmed.starts_with('#') && !trimmed.starts_with("#HttpOnly_")) {
            continue;
        }
        let cols: Vec<&str> = trimmed.split('\t').collect();
        if cols.len() >= 7 {
            count += 1;
            if let Ok(exp) = cols[4].parse::<i64>() {
                if exp > 0 {
                    earliest = Some(earliest.map_or(exp, |prev| prev.min(exp)));
                }
            }
        }
    }
    (count, earliest)
}

#[cfg(windows)]
fn dpapi_protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: plain.len() as u32, pbData: plain.as_ptr() as *mut u8 };
        let entropy = CRYPT_INTEGER_BLOB {
            cbData: DPAPI_ENTROPY.len() as u32,
            pbData: DPAPI_ENTROPY.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = CryptProtectData(
            &input,
            std::ptr::null(),
            &entropy,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        );
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            return Err(tr!(
                "O Windows não conseguiu criptografar os cookies: {err}",
                "Windows couldn't encrypt the cookies: {err}",
                "Windows no pudo cifrar las cookies: {err}"
            ));
        }
        let blob = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Ok(blob)
    }
}

#[cfg(windows)]
fn dpapi_unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: blob.len() as u32, pbData: blob.as_ptr() as *mut u8 };
        let entropy = CRYPT_INTEGER_BLOB {
            cbData: DPAPI_ENTROPY.len() as u32,
            pbData: DPAPI_ENTROPY.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            &entropy,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        );
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            return Err(tr!(
                "O Windows não conseguiu abrir os cookies salvos: {err}",
                "Windows couldn't open the saved cookies: {err}",
                "Windows no pudo abrir las cookies guardadas: {err}"
            ));
        }
        let plain = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        // Wipe the plaintext the OS handed back before releasing it.
        std::ptr::write_bytes(out.pbData, 0, out.cbData as usize);
        LocalFree(out.pbData as _);
        Ok(plain)
    }
}

#[cfg(not(windows))]
fn dpapi_protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
fn dpapi_unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
    Ok(blob.to_vec())
}

/// Formats cookies strictly following the Netscape `cookies.txt` specification expected by `yt-dlp`.
pub fn format_netscape_cookies_txt(entries: &[NetscapeCookieEntry]) -> String {
    let mut out = String::from(
        "# Netscape HTTP Cookie File\n# https://curl.se/docs/http-cookies.html\n# Generated by Boladao\n\n",
    );
    for c in entries {
        let raw_dom = c.domain.trim();
        if raw_dom.is_empty() || c.name.trim().is_empty() {
            continue;
        }
        let (domain, include_sub) = if c.include_subdomains || raw_dom.starts_with('.') {
            let dotted = if raw_dom.starts_with('.') {
                raw_dom.to_string()
            } else {
                format!(".{raw_dom}")
            };
            (dotted, "TRUE")
        } else {
            (raw_dom.to_string(), "FALSE")
        };

        let prefix = if c.http_only { "#HttpOnly_" } else { "" };
        let path = if c.path.is_empty() { "/" } else { c.path.as_str() };
        let secure = if c.secure { "TRUE" } else { "FALSE" };
        // Sanitize tabs/newlines so file structure cannot be broken
        let clean_name = c.name.replace(['\t', '\r', '\n'], "");
        let clean_val = c.value.replace(['\t', '\r', '\n'], "");

        out.push_str(&format!(
            "{prefix}{domain}\t{include_sub}\t{path}\t{secure}\t{}\t{clean_name}\t{clean_val}\n",
            c.expires_unix
        ));
    }
    out
}

pub fn platform_login_urls(platform_id: &str) -> (&'static str, &'static [&'static str]) {
    match platform_id {
        "instagram" => (
            "https://www.instagram.com/accounts/login/",
            &["https://www.instagram.com/"],
        ),
        "tiktok" => (
            "https://www.tiktok.com/login",
            &["https://www.tiktok.com/"],
        ),
        "twitter" => (
            "https://x.com/i/flow/login",
            &["https://x.com/", "https://twitter.com/"],
        ),
        "facebook" => (
            "https://www.facebook.com/login",
            &["https://www.facebook.com/"],
        ),
        "twitch" => (
            "https://www.twitch.tv/login",
            &["https://www.twitch.tv/"],
        ),
        "vimeo" => (
            "https://vimeo.com/log_in",
            &["https://vimeo.com/"],
        ),
        "youtube" | "youtube_music" => (
            "https://accounts.google.com/ServiceLogin?service=youtube&continue=https://www.youtube.com/",
            &["https://www.youtube.com/", "https://music.youtube.com/"],
        ),
        _ => ("https://www.youtube.com/", &["https://www.youtube.com/"]),
    }
}

/// Opens an isolated WebView2 window with a separate `data_directory` and ZERO IPC capabilities.
pub fn open_isolated_login_window(app: &AppHandle, platform_id: &str) -> Result<String, String> {
    let key = if platform_id == "youtube_music" {
        "youtube"
    } else {
        platform_id
    };
    let label = format!("login-{key}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        return Ok(label);
    }

    let (login_url, _) = platform_login_urls(key);
    let parsed_url: url::Url = login_url.parse().map_err(|e: url::ParseError| e.to_string())?;
    let profile_dir = webview_profiles_dir().join(key);
    let _ = fs::create_dir_all(&profile_dir);

    let ua = if key == "youtube" {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0"
    } else {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
    };

    let display_name = match key {
        "youtube" => "YouTube",
        "instagram" => "Instagram",
        "tiktok" => "TikTok",
        "twitter" => "X / Twitter",
        "facebook" => "Facebook",
        "twitch" => "Twitch",
        "vimeo" => "Vimeo",
        other => other,
    };

    WebviewWindowBuilder::new(app, &label, WebviewUrl::External(parsed_url))
        .title(tr!(
            "Entrar em {display_name} — depois clique em 'Concluir login' no Boladão",
            "Sign in to {display_name}, then click 'Finish sign-in' in Boladão",
            "Inicia sesión en {display_name} y después haz clic en 'Finalizar inicio de sesión' en Boladão"
        ))
        .inner_size(520.0, 680.0)
        .center()
        .background_color(tauri::window::Color(14, 14, 16, 255))
        .user_agent(ua)
        .data_directory(profile_dir)
        .build()
        .map_err(|e| tr!("Não foi possível abrir janela de login: {e}", "Couldn't open the sign-in window: {e}", "No se pudo abrir la ventana de inicio de sesión: {e}"))?;

    Ok(label)
}

/// Reads cookies from the open isolated login window (`cookies_for_url` on Tauri 2.12+, outside any webview callback),
/// writes Netscape `cookies.txt`, and closes the login window.
pub fn harvest_login_window_cookies(
    app: &AppHandle,
    platform_id: &str,
) -> Result<ConnectedAccount, String> {
    let key = if platform_id == "youtube_music" {
        "youtube"
    } else {
        platform_id
    };
    let label = format!("login-{key}");
    let win = app
        .get_webview_window(&label)
        .ok_or_else(|| tr!("A janela de login não está aberta. Clique em 'Entrar' primeiro.", "The sign-in window isn't open. Click 'Sign in' first.", "La ventana de inicio de sesión no está abierta. Haz clic en 'Iniciar sesión' primero."))?;

    let (_, target_urls) = platform_login_urls(key);
    let mut entries = Vec::new();
    let mut earliest_exp: Option<i64> = None;

    for raw_u in target_urls {
        let parsed: url::Url = raw_u.parse().map_err(|e: url::ParseError| e.to_string())?;
        let cookies = win
            .cookies_for_url(parsed)
            .map_err(|e| tr!("Falha ao ler cookies da janela: {e}", "Couldn't read the cookies from the window: {e}", "No se pudieron leer las cookies de la ventana: {e}"))?;
        for c in cookies {
            let domain = c.domain().unwrap_or("").to_string();
            if domain.is_empty() {
                continue;
            }
            let exp_unix = c
                .expires_datetime()
                .map(|dt| dt.unix_timestamp().max(0) as u64)
                .unwrap_or(0);
            if exp_unix > 0 {
                let ts = exp_unix as i64;
                earliest_exp = Some(earliest_exp.map_or(ts, |prev| prev.min(ts)));
            }
            entries.push(NetscapeCookieEntry {
                domain,
                include_subdomains: true,
                path: c.path().unwrap_or("/").to_string(),
                secure: c.secure().unwrap_or(true),
                expires_unix: exp_unix,
                http_only: c.http_only().unwrap_or(false),
                name: c.name().to_string(),
                value: c.value().to_string(),
            });
        }
    }

    if entries.is_empty() {
        return Err(tr!("Nenhum cookie encontrado ainda. Conclua o login na janela aberta antes de confirmar.", "No cookies found yet. Finish signing in on the open window before confirming.", "Todavía no hay cookies. Termina de iniciar sesión en la ventana abierta antes de confirmar."));
    }

    let formatted = format_netscape_cookies_txt(&entries);
    save_store_in(&cookies_dir(), key, &formatted)?;
    let _ = win.close();

    Ok(ConnectedAccount {
        platform_id: key.to_string(),
        auth_kind: AccountAuthKind::EmbeddedCookies,
        connected_at: Utc::now(),
        earliest_expiry_ts: earliest_exp,
        cookie_count: entries.len(),
    })
}

/// Imports an existing Netscape `cookies.txt` file chosen by the user.
pub fn import_cookies_txt_file(platform_id: &str, src_path: &Path) -> Result<ConnectedAccount, String> {
    let key = if platform_id == "youtube_music" {
        "youtube"
    } else {
        platform_id
    };
    let raw = fs::read_to_string(src_path)
        .map_err(|e| tr!("Não foi possível ler o arquivo de cookies: {e}", "Couldn't read the cookies file: {e}", "No se pudo leer el archivo de cookies: {e}"))?;

    let (count, earliest_exp) = netscape_cookie_stats(&raw);
    if count == 0 {
        return Err(tr!("Esse arquivo não parece ser um cookies.txt no formato Netscape.", "This file doesn't look like a cookies.txt in Netscape format.", "Este archivo no parece un cookies.txt en formato Netscape."));
    }

    save_store_in(&cookies_dir(), key, &raw)?;

    Ok(ConnectedAccount {
        platform_id: key.to_string(),
        auth_kind: AccountAuthKind::ImportedCookiesFile,
        connected_at: Utc::now(),
        earliest_expiry_ts: earliest_exp,
        cookie_count: count,
    })
}

pub fn logout_account(app: Option<&AppHandle>, platform_id: &str) {
    let key = if platform_id == "youtube_music" {
        "youtube"
    } else {
        platform_id
    };
    if let Some(h) = app {
        if let Some(win) = h.get_webview_window(&format!("login-{key}")) {
            let _ = win.close();
        }
    }
    let dir = cookies_dir();
    let _ = fs::remove_file(store_file(&dir, key));
    let _ = fs::remove_file(legacy_file(&dir, key));
    let _ = fs::remove_dir_all(webview_profiles_dir().join(key));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_valid_netscape_cookies_txt_format() {
        let sample = vec![
            NetscapeCookieEntry {
                domain: ".youtube.com".to_string(),
                include_subdomains: true,
                path: "/".to_string(),
                secure: true,
                expires_unix: 1893456000,
                http_only: true,
                name: "LOGIN_INFO".to_string(),
                value: "AFmmF2swRQIh".to_string(),
            },
            NetscapeCookieEntry {
                domain: "instagram.com".to_string(),
                include_subdomains: false,
                path: "/accounts".to_string(),
                secure: false,
                expires_unix: 0,
                http_only: false,
                name: "csrftoken".to_string(),
                value: "xyz\tclean".to_string(),
            },
        ];

        let txt = format_netscape_cookies_txt(&sample);
        assert!(txt.starts_with("# Netscape HTTP Cookie File\n"));
        let lines: Vec<&str> = txt.lines().collect();
        assert_eq!(
            lines[4],
            "#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1893456000\tLOGIN_INFO\tAFmmF2swRQIh"
        );
        assert_eq!(
            lines[5],
            "instagram.com\tFALSE\t/accounts\tFALSE\t0\tcsrftoken\txyzclean"
        );
    }

    const SAMPLE: &str = "# Netscape HTTP Cookie File\n.instagram.com\tTRUE\t/\tTRUE\t1893456000\tsessionid\tSEGREDO-DA-SESSAO\n";

    #[cfg(windows)]
    #[test]
    fn cookies_are_encrypted_at_rest_and_migrated_from_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(legacy_file(dir.path(), "instagram"), SAMPLE).unwrap();
        fs::create_dir_all(run_dir(dir.path())).unwrap();
        fs::write(run_dir(dir.path()).join("sobra.txt"), SAMPLE).unwrap();

        migrate_legacy_in(dir.path());

        assert!(!legacy_file(dir.path(), "instagram").exists(), "o .txt em texto puro precisa sumir");
        assert!(!run_dir(dir.path()).exists(), "cópias de execução antigas precisam sumir");
        let blob = fs::read(store_file(dir.path(), "instagram")).unwrap();
        assert!(
            !String::from_utf8_lossy(&blob).contains("SEGREDO-DA-SESSAO"),
            "o cookie não pode aparecer no arquivo guardado"
        );
        assert_eq!(load_store_in(dir.path(), "instagram").as_deref(), Some(SAMPLE));
    }

    #[cfg(windows)]
    #[test]
    fn lease_keeps_refreshed_cookies_and_deletes_the_plaintext_copy() {
        let dir = tempfile::tempdir().unwrap();
        save_store_in(dir.path(), "youtube", SAMPLE).unwrap();

        // yt-dlp refreshed the session: the new jar replaces the stored one.
        let refreshed = format!("{SAMPLE}.youtube.com\tTRUE\t/\tTRUE\t1893456000\tPSIDTS\tnovo\n");
        let lease = CookieLease::open_in(dir.path(), "youtube").unwrap();
        let copy = lease.path().to_path_buf();
        assert_eq!(fs::read_to_string(&copy).unwrap(), SAMPLE);
        fs::write(&copy, &refreshed).unwrap();
        drop(lease);
        assert!(!copy.exists(), "a cópia em texto puro só existe enquanto o yt-dlp roda");
        assert_eq!(load_store_in(dir.path(), "youtube").as_deref(), Some(refreshed.as_str()));

        // A run killed mid-write leaves a truncated file: the stored cookies must survive.
        let lease = CookieLease::open_in(dir.path(), "youtube").unwrap();
        fs::write(lease.path(), "").unwrap();
        drop(lease);
        assert_eq!(load_store_in(dir.path(), "youtube").as_deref(), Some(refreshed.as_str()));
    }
}
