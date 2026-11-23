//! Self-update through GitHub Releases, with no server of our own.
//!
//! Every release carries one manifest per system (version, download URL and a minisign signature
//! of the package): `latest.json` for `Boladao-Instalador.exe` on Windows (scripts/build-installer.ps1),
//! `latest-linux-x86_64.json` for the AppImage and `latest-macos.json` for the `.app` packed as
//! `.tar.gz` (both from the release workflow). The app reads its manifest, downloads the package
//! through the bandwidth limiter, checks the signature against `PUBLIC_KEY` and keeps the file in
//! its data folder (`updates`). The update is applied when the user restarts from the banner or,
//! at the latest, the next time the app opens.

#[cfg(windows)]
use crate::installer::copy_clean_executable;
use crate::paths::app_local_dir;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// This system's manifest. GitHub redirects `releases/latest/download/<asset>` to the newest
/// published release.
#[cfg(windows)]
const MANIFEST_NAME: &str = "latest.json";
#[cfg(target_os = "linux")]
const MANIFEST_NAME: &str = "latest-linux-x86_64.json";
#[cfg(target_os = "macos")]
const MANIFEST_NAME: &str = "latest-macos.json";

/// Extension of the downloaded package.
#[cfg(windows)]
const PACKAGE_EXT: &str = "exe";
#[cfg(target_os = "linux")]
const PACKAGE_EXT: &str = "AppImage";
#[cfg(target_os = "macos")]
const PACKAGE_EXT: &str = "app.tar.gz";

fn manifest_url() -> String {
    format!("https://github.com/aekasota/BOLADAO/releases/latest/download/{MANIFEST_NAME}")
}
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

/// What an update replaces: the `.exe` on Windows, the AppImage on Linux and the `.app` bundle on
/// macOS. `None` when this copy does not update itself: a development build, or a `.deb` install,
/// which the system's package manager updates.
pub fn install_target() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::current_exe().ok()
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file())
    }
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().ok()?;
        exe.ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .map(Path::to_path_buf)
    }
}

/// Where the replaced version stays until the next start: `<exe>.exe.old` on Windows,
/// `<AppImage>.old` and `<bundle>.app.old` elsewhere.
fn old_path(target: &Path) -> PathBuf {
    if cfg!(windows) {
        return target.with_extension("exe.old");
    }
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    target.with_file_name(name)
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
    if install_target().is_none() {
        return Ok(None);
    }
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("Boladao/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15));
    if let Some(proxy) = proxy_url {
        // Through the local limiter, so the global bandwidth limit and Game Mode also apply here.
        builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|e| e.to_string())?);
    }
    let client = builder.build().map_err(|e| e.to_string())?;

    let resp = client
        .get(manifest_url())
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
        .map_err(|e| format!("{MANIFEST_NAME} inválido: {e}"))?;

    if !is_newer(&manifest.version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    if !manifest.url.starts_with(DOWNLOAD_PREFIX) {
        return Err(format!("O {MANIFEST_NAME} aponta para fora das releases do Boladão."));
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
    let file = dir.join(format!("Boladao-{}.{PACKAGE_EXT}", manifest.version));
    let partial = dir.join(format!("Boladao-{}.part", manifest.version));
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

/// Replaces the running version with the pending update and, if asked, starts the new version.
/// A running `.exe`, AppImage or `.app` can be renamed (not deleted), so the current one becomes
/// `<target>.old` and is removed on the next start. Returns `Ok(true)` when the update was installed.
pub fn apply_pending(relaunch: bool) -> Result<bool, String> {
    let Some(pending) = read_pending() else {
        return Ok(false);
    };
    let Some(target) = install_target() else {
        discard_pending();
        return Ok(false);
    };
    let installed = install_over(&target, &pending.file, &pending.signature);
    if installed.is_err() {
        discard_pending();
    }
    installed?;
    discard_pending();

    if relaunch {
        // The new version waits for this process to close; otherwise it would find the old one
        // still open and hand control back to it.
        let pid = std::process::id().to_string();
        #[cfg(not(target_os = "macos"))]
        let launched = std::process::Command::new(&target)
            .arg(crate::instance::WAIT_FOR_PID_ARG)
            .arg(pid)
            .current_dir(target.parent().map(Path::to_path_buf).unwrap_or_default())
            .spawn();
        #[cfg(target_os = "macos")]
        let launched = std::process::Command::new("/usr/bin/open")
            .arg("-n")
            .arg(&target)
            .arg("--args")
            .arg(crate::instance::WAIT_FOR_PID_ARG)
            .arg(pid)
            .spawn();
        launched.map_err(|e| format!("Atualizado, mas não foi possível reabrir o Boladão: {e}"))?;
    }
    Ok(true)
}

/// Verifies `update_file` (again: it sat on disk since the download) and puts it in place of
/// `target`, keeping the replaced version as `<target>.old`. Rolls back if the copy fails.
fn install_over(target: &Path, update_file: &Path, signature: &str) -> Result<(), String> {
    let data = fs::read(update_file).map_err(|e| e.to_string())?;
    verify_signature(&data, signature)?;

    #[cfg(target_os = "macos")]
    {
        // The `.app` comes packed: unpack it next to the old one (same disk, so the swap is a rename).
        let parent = target.parent().ok_or("pasta do aplicativo desconhecida")?;
        let staging = parent.join(".boladao-update");
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging).map_err(|e| format!("Não foi possível preparar a atualização: {e}"))?;
        let unpacked = tar::Archive::new(flate2::read::GzDecoder::new(data.as_slice())).unpack(&staging);
        drop(data);
        let new_app = unpacked.ok().and_then(|_| {
            fs::read_dir(&staging)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "app"))
        });
        let Some(new_app) = new_app else {
            let _ = fs::remove_dir_all(&staging);
            return Err("O pacote da atualização não tem o aplicativo.".to_string());
        };
        let old = old_path(target);
        let _ = fs::remove_dir_all(&old);
        fs::rename(target, &old).map_err(|e| format!("Não foi possível trocar o aplicativo: {e}"))?;
        if let Err(e) = fs::rename(&new_app, target) {
            let _ = fs::rename(&old, target);
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("Não foi possível trocar o aplicativo: {e}"));
        }
        let _ = fs::remove_dir_all(&staging);
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        drop(data);
        let old = old_path(target);
        let _ = fs::remove_file(&old);
        fs::rename(target, &old).map_err(|e| format!("Não foi possível trocar o executável: {e}"))?;
        if let Err(e) = put_in_place(update_file, target) {
            let _ = fs::rename(&old, target);
            return Err(e);
        }
        Ok(())
    }
}

/// Copies the downloaded executable to its place: without the installer mark on Windows, with the
/// executable bit on Linux.
#[cfg(not(target_os = "macos"))]
fn put_in_place(update_file: &Path, target: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        copy_clean_executable(update_file, target)
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::copy(update_file, target).map_err(|e| format!("Não foi possível trocar o executável: {e}"))?;
        fs::set_permissions(target, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())
    }
}

/// Removes the previous version left behind by an update.
pub fn cleanup_previous_binary() {
    if let Some(target) = install_target() {
        let old = old_path(&target);
        if old.is_dir() {
            let _ = fs::remove_dir_all(&old);
        } else {
            let _ = fs::remove_file(&old);
        }
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn swaps_the_executable_only_for_a_signed_update() {
        let signed = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSSFlNWDBiajhBNUZ3RnJyN2ZMdVk3UmlWMVVld1BtVVJNR2tSSTlFYlVqVktabzVxeEdQMHpzTTk0SW5LWm1IVC9aUVJZSkFPSTJkdTY1MXhMS2poOElWdXN2RkpKWXd3PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk1MzQ1MDIzCWZpbGU6c2lndGVzdC5iaW4KWWdkQ0xaN2oyNjRrNlBjTTA5RFBwSDBmbkUraEtsbEZRaG8rb2lsa2N2U1NQUDdKOTJRZ04ya3NSb3ZPZld4TElxYlRLR1FNcVU0bzhaU0dNdFF6Qnc9PQo=";
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(if cfg!(windows) { "Boladão.exe" } else { "Boladão.AppImage" });
        let update = dir.path().join(format!("Boladao-9.9.9.{PACKAGE_EXT}"));

        // Tampered download: nothing changes.
        fs::write(&target, b"versao antiga").unwrap();
        fs::write(&update, b"teste com virus\n").unwrap();
        assert!(install_over(&target, &update, signed).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"versao antiga");

        // Signed download: the new binary takes the place, the old one is kept as `.old`.
        fs::write(&update, b"teste\n").unwrap();
        install_over(&target, &update, signed).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"teste\n");
        assert_eq!(fs::read(old_path(&target)).unwrap(), b"versao antiga");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o111, 0o111);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn keeps_the_app_when_the_package_is_not_signed() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Boladão.app");
        fs::create_dir_all(app.join("Contents")).unwrap();
        let update = dir.path().join("Boladao-9.9.9.app.tar.gz");
        fs::write(&update, b"pacote adulterado").unwrap();
        assert!(install_over(&app, &update, "nao-e-base64").is_err());
        assert!(app.join("Contents").is_dir());
    }
}
