//! Self-update through GitHub Releases, with no server of our own.
//!
//! Every release carries `latest.json` (version, download URL and a minisign signature of
//! `Boladao-Instalador.exe`), produced by `scripts/build-installer.ps1`. The app reads it, downloads
//! the installer through the bandwidth limiter, checks the signature against `PUBLIC_KEY` and keeps
//! the file in `%LOCALAPPDATA%\Boladao\updates`. The update is applied when the user restarts from
//! the banner or, at the latest, the next time the app opens.

use crate::installer::copy_clean_executable;
use crate::paths::app_local_dir;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// GitHub redirects `releases/latest/download/<asset>` to the newest published release.
const MANIFEST_URL: &str = "https://github.com/aekasota/BOLADAO/releases/latest/download/latest.json";
/// Updates may only come from this repository's releases.
const DOWNLOAD_PREFIX: &str = "https://github.com/aekasota/BOLADAO/releases/download/";
/// Public half of the minisign key that signs releases. The private half never enters the
/// repository: it lives in `%USERPROFILE%\.boladao\updater.key` on the release machine.
/// Rotating it requires a release installed by hand, since older builds only trust this key.
const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEU0MDAzRjZFRjRDNTYwNDcKUldSSFlNWDBiajhBNVB2OEtrRml6M0Z2clVONWMyTlFjazJNdlpJaXNuUmljYllPbVdYZmY3cG8K";

#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
    url: String,
    signature: String,
    #[serde(default)]
    notes: Option<String>,
}

/// A downloaded, signature-checked update waiting to be applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUpdate {
    pub version: String,
    pub notes: Option<String>,
    file: PathBuf,
    signature: String,
}

fn updates_dir() -> PathBuf {
    app_local_dir().join("updates")
}

fn pending_path() -> PathBuf {
    updates_dir().join("pending.json")
}

fn version_parts(v: &str) -> [u64; 3] {
    let mut parts = [0u64; 3];
    for (slot, piece) in parts
        .iter_mut()
        .zip(v.trim().trim_start_matches('v').split(['.', '-', '+']))
    {
        *slot = piece.parse().unwrap_or(0);
    }
    parts
}

/// `true` when `candidate` is a strictly newer `MAJOR.MINOR.PATCH` than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    version_parts(candidate) > version_parts(current)
}

fn verify_signature(data: &[u8], signature_b64: &str) -> Result<(), String> {
    let decode = |text: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(|| "assinatura em formato inválido".to_string())
    };
    let key = minisign_verify::PublicKey::decode(&decode(PUBLIC_KEY)?)
        .map_err(|e| format!("chave pública inválida: {e}"))?;
    let signature = minisign_verify::Signature::decode(&decode(signature_b64)?)
        .map_err(|e| format!("assinatura inválida: {e}"))?;
    key.verify(data, &signature, true)
        .map_err(|_| "a assinatura não confere com o arquivo baixado".to_string())
}

pub fn read_pending() -> Option<PendingUpdate> {
    let raw = fs::read_to_string(pending_path()).ok()?;
    let pending: PendingUpdate = serde_json::from_str(&raw).ok()?;
    if pending.file.is_file() && is_newer(&pending.version, env!("CARGO_PKG_VERSION")) {
        Some(pending)
    } else {
        discard_pending();
        None
    }
}

fn discard_pending() {
    let _ = fs::remove_dir_all(updates_dir());
}

/// Checks the latest release and, when it is newer, downloads and verifies it.
/// Returns the update that is ready to apply, if any.
pub async fn check_and_download(proxy_url: Option<String>) -> Result<Option<PendingUpdate>, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("Boladao/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15));
    if let Some(proxy) = proxy_url {
        // Through the local limiter, so the global bandwidth limit and Game Mode also apply here.
        builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|e| e.to_string())?);
    }
    let client = builder.build().map_err(|e| e.to_string())?;

    let resp = client
        .get(MANIFEST_URL)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Falha consultando atualizações: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None); // release without manifest (published before the updater existed)
    }
    let text = resp
        .error_for_status()
        .map_err(|e| format!("Falha consultando atualizações: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let manifest: Manifest = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("latest.json inválido: {e}"))?;

    if !is_newer(&manifest.version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    if !manifest.url.starts_with(DOWNLOAD_PREFIX) {
        return Err("O latest.json aponta para fora das releases do Boladão.".to_string());
    }
    if let Some(pending) = read_pending() {
        if pending.version == manifest.version {
            return Ok(Some(pending));
        }
    }

    let bytes = client
        .get(&manifest.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Falha baixando a atualização: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Falha baixando a atualização: {e}"))?;
    verify_signature(&bytes, &manifest.signature)?;

    let dir = updates_dir();
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("Boladao-{}.exe", manifest.version));
    let partial = file.with_extension("exe.part");
    fs::write(&partial, &bytes).map_err(|e| format!("Erro salvando a atualização: {e}"))?;
    fs::rename(&partial, &file).map_err(|e| format!("Erro salvando a atualização: {e}"))?;

    let pending = PendingUpdate {
        version: manifest.version,
        notes: manifest.notes,
        file,
        signature: manifest.signature,
    };
    let json = serde_json::to_string(&pending).map_err(|e| e.to_string())?;
    fs::write(pending_path(), json).map_err(|e| e.to_string())?;
    Ok(Some(pending))
}

/// Replaces the running executable with the pending update and, if asked, starts the new version.
/// Windows lets a running `.exe` be renamed (not deleted), so the current file becomes `<exe>.old`
/// and is removed on the next start. Returns `Ok(true)` when the update was installed.
pub fn apply_pending(relaunch: bool) -> Result<bool, String> {
    let Some(pending) = read_pending() else {
        return Ok(false);
    };
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let installed = install_over(&current, &pending.file, &pending.signature);
    if installed.is_err() {
        discard_pending();
    }
    installed?;
    discard_pending();

    if relaunch {
        let dir = current.parent().map(Path::to_path_buf).unwrap_or_default();
        std::process::Command::new(&current)
            .current_dir(dir)
            .spawn()
            .map_err(|e| format!("Atualizado, mas não foi possível reabrir o Boladão: {e}"))?;
    }
    Ok(true)
}

/// Verifies `update_file` (again: it sat on disk since the download) and puts it in place of
/// `target`, keeping the replaced binary as `<target>.old`. Rolls back if the copy fails.
fn install_over(target: &Path, update_file: &Path, signature: &str) -> Result<(), String> {
    let data = fs::read(update_file).map_err(|e| e.to_string())?;
    verify_signature(&data, signature)?;
    drop(data);

    let old = target.with_extension("exe.old");
    let _ = fs::remove_file(&old);
    fs::rename(target, &old).map_err(|e| format!("Não foi possível trocar o executável: {e}"))?;
    if let Err(e) = copy_clean_executable(update_file, target) {
        let _ = fs::rename(&old, target);
        return Err(e);
    }
    Ok(())
}

/// Removes the previous executable left behind by an update.
pub fn cleanup_previous_binary() {
    if let Ok(current) = std::env::current_exe() {
        let _ = fs::remove_file(current.with_extension("exe.old"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_release_versions() {
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(is_newer("2.0.0", "1.99.99"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("1.0.9", "1.1.0"));
    }

    #[test]
    fn accepts_only_files_signed_by_the_release_key() {
        // Signature of the bytes "teste\n" made with the release private key (`tauri signer sign`).
        // It also proves that `PUBLIC_KEY` matches the key the build script signs with.
        let signed = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSSFlNWDBiajhBNUZ3RnJyN2ZMdVk3UmlWMVVld1BtVVJNR2tSSTlFYlVqVktabzVxeEdQMHpzTTk0SW5LWm1IVC9aUVJZSkFPSTJkdTY1MXhMS2poOElWdXN2RkpKWXd3PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk1MzQ1MDIzCWZpbGU6c2lndGVzdC5iaW4KWWdkQ0xaN2oyNjRrNlBjTTA5RFBwSDBmbkUraEtsbEZRaG8rb2lsa2N2U1NQUDdKOTJRZ04ya3NSb3ZPZld4TElxYlRLR1FNcVU0bzhaU0dNdFF6Qnc9PQo=";
        assert!(verify_signature(b"teste\n", signed).is_ok());
        assert!(verify_signature(b"teste adulterado\n", signed).is_err());
        assert!(verify_signature(b"teste\n", "nao-e-base64").is_err());
    }

    #[test]
    fn swaps_the_executable_only_for_a_signed_update() {
        let signed = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSSFlNWDBiajhBNUZ3RnJyN2ZMdVk3UmlWMVVld1BtVVJNR2tSSTlFYlVqVktabzVxeEdQMHpzTTk0SW5LWm1IVC9aUVJZSkFPSTJkdTY1MXhMS2poOElWdXN2RkpKWXd3PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk1MzQ1MDIzCWZpbGU6c2lndGVzdC5iaW4KWWdkQ0xaN2oyNjRrNlBjTTA5RFBwSDBmbkUraEtsbEZRaG8rb2lsa2N2U1NQUDdKOTJRZ04ya3NSb3ZPZld4TElxYlRLR1FNcVU0bzhaU0dNdFF6Qnc9PQo=";
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Boladão.exe");
        let update = dir.path().join("Boladao-9.9.9.exe");

        // Tampered download: nothing changes.
        fs::write(&target, b"versao antiga").unwrap();
        fs::write(&update, b"teste com virus\n").unwrap();
        assert!(install_over(&target, &update, signed).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"versao antiga");

        // Signed download: the new binary takes the place, the old one is kept as `.exe.old`.
        fs::write(&update, b"teste\n").unwrap();
        install_over(&target, &update, signed).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"teste\n");
        assert_eq!(fs::read(target.with_extension("exe.old")).unwrap(), b"versao antiga");
    }
}
