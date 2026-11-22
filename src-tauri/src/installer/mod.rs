use crate::bins::BinManager;
use crate::library::LibraryStore;
use crate::paths::{
    app_config_dir, app_local_dir, default_install_dir, default_music_root, default_video_root,
    desktop_dir, start_menu_programs_dir, APP_NAME,
};
use crate::settings::load_or_init_settings;
use crate::winproc::CREATE_NO_WINDOW_FLAG;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

pub const INSTALLER_MAGIC: &[u8; 16] = b"BOLADAO_INSTALL!";

static EMBEDDED_WEBVIEW2_SETUP: &[u8] =
    include_bytes!("../../../vendor/bin/webview2/MicrosoftEdgeWebview2Setup.exe");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LaunchMode {
    App,
    Installer,
    Uninstaller { target_dir: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallerDefaults {
    pub default_install_dir: PathBuf,
    pub already_installed: bool,
    pub app_version: String,
    pub music_dir: PathBuf,
    pub video_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallerProgressPayload {
    pub pct: f32,
    pub stage: String,
    pub done: bool,
    pub installed_exe: Option<PathBuf>,
    pub error: Option<String>,
}

pub fn has_installer_magic(exe_path: &Path) -> bool {
    let Ok(mut f) = File::open(exe_path) else {
        return false;
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return false;
    };
    if len < 16 {
        return false;
    }
    if f.seek(SeekFrom::End(-16)).is_err() {
        return false;
    }
    let mut tail = [0u8; 16];
    if f.read_exact(&mut tail).is_err() {
        return false;
    }
    &tail == INSTALLER_MAGIC
}

pub fn detect_launch_mode() -> LaunchMode {
    let args: Vec<String> = env::args().collect();

    // Step 2 of self-uninstall: running from %TEMP% with target install folder argument
    if let Some(pos) = args.iter().position(|a| a == "--uninstall-from-temp") {
        let target = args
            .get(pos + 1)
            .map(PathBuf::from)
            .unwrap_or_else(default_install_dir);
        return LaunchMode::Uninstaller { target_dir: target };
    }

    // Step 1 of self-uninstall: copy self to %TEMP% so <InstallDir>\Boladão.exe is unlocked, then relaunch
    if args.iter().any(|a| a == "--uninstall") {
        if let Ok(cur_exe) = env::current_exe() {
            let install_dir = cur_exe
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(default_install_dir);
            let temp_exe = env::temp_dir().join(format!(
                "Boladao-Uninstall-{}.exe",
                Uuid::new_v4().simple()
            ));
            if copy_clean_executable(&cur_exe, &temp_exe).is_ok() {
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    let _ = std::process::Command::new(&temp_exe)
                        .arg("--uninstall-from-temp")
                        .arg(&install_dir)
                        .creation_flags(CREATE_NO_WINDOW_FLAG)
                        .spawn();
                    std::process::exit(0);
                }
            }
            return LaunchMode::Uninstaller {
                target_dir: install_dir,
            };
        }
    }

    if args.iter().any(|a| a == "--installer") {
        return LaunchMode::Installer;
    }

    if let Ok(cur_exe) = env::current_exe() {
        if has_installer_magic(&cur_exe) {
            return LaunchMode::Installer;
        }
        if let Some(stem) = cur_exe.file_stem().and_then(|s| s.to_str()) {
            let lower = stem.to_lowercase();
            if (lower.contains("instal") || lower.contains("setup")) && !lower.contains("uninstal")
            {
                return LaunchMode::Installer;
            }
        }
    }

    LaunchMode::App
}

/// Ensures Microsoft Edge WebView2 Runtime is installed; if missing, extracts and runs the embedded official bootstrapper silently.
pub fn ensure_webview2_or_bootstrap() -> bool {
    if crate::winproc::ensure_webview2_installed() {
        return true;
    }

    let bootstrapper_path = env::temp_dir().join("Boladao-WebView2Setup.exe");
    if fs::write(&bootstrapper_path, EMBEDDED_WEBVIEW2_SETUP).is_ok() {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new(&bootstrapper_path)
                .args(["/silent", "/install"])
                .creation_flags(CREATE_NO_WINDOW_FLAG)
                .status();
        }
        let _ = fs::remove_file(&bootstrapper_path);
    }

    crate::winproc::ensure_webview2_installed()
}

/// Copies `src_exe` to `dst_exe`, stripping the 16-byte `BOLADAO_INSTALL!` trailer if present so the installed binary opens in normal App mode.
pub fn copy_clean_executable(src_exe: &Path, dst_exe: &Path) -> Result<(), String> {
    let meta = fs::metadata(src_exe).map_err(|e| tr!("Erro lendo executável: {e}", "Error reading the executable: {e}", "Error al leer el ejecutable: {e}"))?;
    let copy_len = if has_installer_magic(src_exe) {
        meta.len().saturating_sub(16)
    } else {
        meta.len()
    };

    if let Some(parent) = dst_exe.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            let dir = parent.display();
            tr!("Erro criando pasta {dir}: {e}", "Error creating folder {dir}: {e}", "Error al crear la carpeta {dir}: {e}")
        })?;
    }

    let tmp_dst = dst_exe.with_extension("exe.installing");
    {
        let src_file = File::open(src_exe).map_err(|e| tr!("Erro abrindo origem: {e}", "Error opening the source: {e}", "Error al abrir el origen: {e}"))?;
        let dst_file = File::create(&tmp_dst)
            .map_err(|e| {
                let path = tmp_dst.display();
                tr!("Erro criando {path}: {e}", "Error creating {path}: {e}", "Error al crear {path}: {e}")
            })?;
        let mut reader = BufReader::with_capacity(512 * 1024, src_file.take(copy_len));
        let mut writer = BufWriter::with_capacity(512 * 1024, dst_file);
        std::io::copy(&mut reader, &mut writer)
            .map_err(|e| tr!("Erro copiando binário: {e}", "Error copying the binary: {e}", "Error al copiar el binario: {e}"))?;
        writer
            .flush()
            .map_err(|e| tr!("Erro gravando binário no disco: {e}", "Error writing the binary to disk: {e}", "Error al escribir el binario en el disco: {e}"))?;
    }

    if dst_exe.exists() && fs::remove_file(dst_exe).is_err() {
        // Attempt to close running app instance in destination if upgrading
        close_other_boladao_instances();
        std::thread::sleep(std::time::Duration::from_millis(350));
        let _ = fs::remove_file(dst_exe);
    }

    fs::rename(&tmp_dst, dst_exe).map_err(|e| {
        let path = dst_exe.display();
        tr!(
            "Não foi possível atualizar {path} (feche o Boladão se estiver aberto): {e}",
            "Couldn't update {path} (close Boladão if it is open): {e}",
            "No se pudo actualizar {path} (cierra Boladão si está abierto): {e}"
        )
    })
}

fn close_other_boladao_instances() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let my_pid = std::process::id();
        for img in ["Boladão.exe", "boladao.exe", "Boladao.exe"] {
            let _ = std::process::Command::new("taskkill.exe")
                .args(["/F", "/FI", &format!("PID ne {my_pid}"), "/IM", img])
                .creation_flags(CREATE_NO_WINDOW_FLAG)
                .status();
        }
    }
}

pub fn get_installer_defaults_info() -> InstallerDefaults {
    let existing = read_registered_install_dir();
    let already_installed = existing
        .as_ref()
        .map(|p| p.join("Boladão.exe").is_file() || p.join("boladao.exe").is_file())
        .unwrap_or(false);
    let default_dir = existing.unwrap_or_else(default_install_dir);

    InstallerDefaults {
        default_install_dir: default_dir,
        already_installed,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        music_dir: default_music_root(),
        video_dir: default_video_root(),
    }
}

fn emit_progress(
    app: &AppHandle,
    pct: f32,
    stage: impl Into<String>,
    done: bool,
    installed_exe: Option<PathBuf>,
    error: Option<String>,
) {
    let _ = app.emit(
        "installer:progress",
        InstallerProgressPayload {
            pct,
            stage: stage.into(),
            done,
            installed_exe,
            error,
        },
    );
}

pub async fn run_install_pipeline(
    app: AppHandle,
    install_dir: PathBuf,
    create_desktop_shortcut: bool,
    create_start_menu_shortcut: bool,
) -> Result<PathBuf, String> {
    emit_progress(
        &app,
        0.06,
        &tr!("Preparando diretório de instalação...", "Preparing the install folder...", "Preparando la carpeta de instalación..."),
        false,
        None,
        None,
    );

    let install_dir_clone = install_dir.clone();
    let target_exe = tokio::task::spawn_blocking(move || -> Result<PathBuf, String> {
        fs::create_dir_all(&install_dir_clone)
            .map_err(|e| {
                let dir = install_dir_clone.display();
                tr!("Não foi possível criar {dir}: {e}", "Couldn't create {dir}: {e}", "No se pudo crear {dir}: {e}")
            })?;
        let cur_exe =
            env::current_exe().map_err(|e| tr!("Erro localizando instalador: {e}", "Error locating the installer: {e}", "Error al localizar el instalador: {e}"))?;
        let dst_exe = install_dir_clone.join("Boladão.exe");
        // Clean legacy lowercase binary if upgrading from older NSIS install
        let legacy_lower = install_dir_clone.join("boladao.exe");
        if legacy_lower.is_file() && legacy_lower != dst_exe {
            // Note: Windows NTFS is case-insensitive, so rename/replace handles casing cleanly
            let _ = fs::remove_file(&legacy_lower);
        }
        copy_clean_executable(&cur_exe, &dst_exe)?;
        Ok(dst_exe)
    })
    .await
    .map_err(|e| e.to_string())??;

    emit_progress(
        &app,
        0.28,
        &tr!(
            "Criando pastas Músicas\\Boladão, Vídeos\\Boladão e banco local...",
            "Creating Music\\Boladão, Videos\\Boladão and the local database...",
            "Creando las carpetas Música\\Boladão, Videos\\Boladão y la base local..."
        ),
        false,
        None,
        None,
    );

    let ytdlp_channel = tokio::task::spawn_blocking(|| {
        let cfg = load_or_init_settings();
        let _ = fs::create_dir_all(&cfg.music_root);
        let _ = fs::create_dir_all(&cfg.video_root);
        let _ = LibraryStore::open_default();
        cfg.ytdlp_channel
    })
    .await
    .map_err(|e| e.to_string())?;

    emit_progress(
        &app,
        0.46,
        &tr!("Extraindo motores embutidos (yt-dlp, Deno, FFmpeg 9.0.2)...", "Extracting built-in engines (yt-dlp, Deno, FFmpeg 9.0.2)...", "Extrayendo los motores incluidos (yt-dlp, Deno, FFmpeg 9.0.2)..."),
        false,
        None,
        None,
    );

    let bins = BinManager::new();
    bins.ensure_ready(Some(app.clone()), ytdlp_channel).await?;

    emit_progress(
        &app,
        0.86,
        &tr!("Criando atalhos e registrando nos Aplicativos do Windows...", "Creating shortcuts and registering in Windows Apps...", "Creando accesos directos y registrando en las Aplicaciones de Windows..."),
        false,
        None,
        None,
    );

    let exe_for_shortcuts = target_exe.clone();
    let dir_for_reg = install_dir.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        if create_desktop_shortcut {
            let lnk = desktop_dir().join(format!("{APP_NAME}.lnk"));
            create_windows_shortcut(&lnk, &exe_for_shortcuts, &dir_for_reg)?;
        }
        if create_start_menu_shortcut {
            let sm_dir = start_menu_programs_dir();
            let _ = fs::create_dir_all(&sm_dir);
            let lnk = sm_dir.join(format!("{APP_NAME}.lnk"));
            create_windows_shortcut(&lnk, &exe_for_shortcuts, &dir_for_reg)?;
        }
        register_windows_uninstaller(&dir_for_reg, &exe_for_shortcuts)?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;

    emit_progress(
        &app,
        1.0,
        &tr!("Instalação concluída! Tudo pronto para usar.", "Installation complete! Everything is ready.", "¡Instalación completada! Todo listo para usar."),
        true,
        Some(target_exe.clone()),
        None,
    );

    Ok(target_exe)
}

pub async fn run_uninstall_pipeline(
    target_dir: PathBuf,
    remove_user_data: bool,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        close_other_boladao_instances();
        std::thread::sleep(std::time::Duration::from_millis(300));

        // 1. Remove installed binaries and old NSIS uninstaller if any
        for fname in ["Boladão.exe", "boladao.exe", "uninstall.exe"] {
            let p = target_dir.join(fname);
            if p.is_file() {
                let _ = fs::remove_file(&p);
            }
        }
        let _ = fs::remove_dir(&target_dir);

        // 2. Remove shortcuts
        let _ = fs::remove_file(desktop_dir().join(format!("{APP_NAME}.lnk")));
        let _ = fs::remove_file(start_menu_programs_dir().join(format!("{APP_NAME}.lnk")));

        // 3. Remove extracted ~360 MB internal binaries & caches (always preserving Músicas/Vídeos!)
        let local = app_local_dir();
        let _ = fs::remove_dir_all(local.join("bin"));
        let _ = fs::remove_dir_all(local.join("cache"));

        if remove_user_data {
            let _ = fs::remove_dir_all(app_config_dir());
            let _ = fs::remove_dir_all(local);
        }

        // 4. Remove Windows Uninstall registry entry
        unregister_windows_uninstaller();

        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

pub fn launch_installed_app_and_close(app: &AppHandle, installed_exe: &Path) -> Result<(), String> {
    if !installed_exe.is_file() {
        return Err(tr!("Executável instalado não encontrado.", "Installed executable not found.", "No se encontró el ejecutable instalado."));
    }
    let parent = installed_exe
        .parent()
        .unwrap_or_else(|| Path::new("."));
    std::process::Command::new(installed_exe)
        .current_dir(parent)
        .spawn()
        .map_err(|e| tr!("Falha ao iniciar Boladão: {e}", "Couldn't start Boladão: {e}", "No se pudo iniciar Boladão: {e}"))?;
    app.exit(0);
    Ok(())
}

pub fn close_and_cleanup_temp_uninstaller(app: &AppHandle) {
    #[cfg(windows)]
    {
        if let Ok(cur_exe) = env::current_exe() {
            let temp_root = env::temp_dir();
            if cur_exe.starts_with(&temp_root) {
                use std::os::windows::process::CommandExt;
                let cmd_line = format!(
                    "ping 127.0.0.1 -n 2 > nul & del /f /q \"{}\"",
                    cur_exe.display()
                );
                let _ = std::process::Command::new("cmd.exe")
                    .args(["/C", &cmd_line])
                    .creation_flags(CREATE_NO_WINDOW_FLAG)
                    .spawn();
            }
        }
    }
    app.exit(0);
}

fn create_windows_shortcut(lnk_path: &Path, target_exe: &Path, work_dir: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let ps_script = r#"
$ws = New-Object -ComObject WScript.Shell
$s = $ws.CreateShortcut($env:BOLADAO_LNK)
$s.TargetPath = $env:BOLADAO_EXE
$s.WorkingDirectory = $env:BOLADAO_DIR
$s.IconLocation = "$($env:BOLADAO_EXE),0"
$s.Description = $env:BOLADAO_DESC
$s.Save()
"#;
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", ps_script])
            .env("BOLADAO_LNK", lnk_path)
            .env("BOLADAO_EXE", target_exe)
            .env("BOLADAO_DIR", work_dir)
            .env(
                "BOLADAO_DESC",
                tr!(
                    "Boladão — Downloader de vídeo e áudio",
                    "Boladão — Video and audio downloader",
                    "Boladão — Descargador de video y audio"
                ),
            )
            .creation_flags(CREATE_NO_WINDOW_FLAG)
            .status()
            .map_err(|e| tr!("Erro criando atalho: {e}", "Error creating the shortcut: {e}", "Error al crear el acceso directo: {e}"))?;
        if !status.success() {
            return Err(tr!("Falha ao gravar atalho .lnk do Windows.", "Couldn't write the Windows .lnk shortcut.", "No se pudo guardar el acceso directo .lnk de Windows."));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (lnk_path, target_exe, work_dir);
        Ok(())
    }
}

#[cfg(windows)]
fn wide_null(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn set_reg_sz(hkey: windows_sys::Win32::System::Registry::HKEY, name: &str, val: &str) {
    unsafe {
        use windows_sys::Win32::System::Registry::{RegSetValueExW, REG_SZ};
        let wname = wide_null(name);
        let wval = wide_null(val);
        let byte_len = (wval.len() * 2) as u32;
        let _ = RegSetValueExW(
            hkey,
            wname.as_ptr(),
            0,
            REG_SZ,
            wval.as_ptr() as *const u8,
            byte_len,
        );
    }
}

#[cfg(windows)]
fn set_reg_dword(hkey: windows_sys::Win32::System::Registry::HKEY, name: &str, val: u32) {
    unsafe {
        use windows_sys::Win32::System::Registry::{RegSetValueExW, REG_DWORD};
        let wname = wide_null(name);
        let bytes = val.to_le_bytes();
        let _ = RegSetValueExW(hkey, wname.as_ptr(), 0, REG_DWORD, bytes.as_ptr(), 4);
    }
}

fn register_windows_uninstaller(install_dir: &Path, target_exe: &Path) -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE,
        };
        let subkey = wide_null("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Boladão");
        let mut hkey: HKEY = std::ptr::null_mut();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null(),
            &mut hkey,
            std::ptr::null_mut(),
        );
        if status != 0 {
            return Err(tr!(
                "Erro registrando aplicativo no Windows (código {status}).",
                "Error registering the app in Windows (code {status}).",
                "Error al registrar la aplicación en Windows (código {status})."
            ));
        }

        let icon_str = format!("\"{}\",0", target_exe.display());
        let uninst_str = format!("\"{}\" --uninstall", target_exe.display());
        set_reg_sz(hkey, "DisplayName", APP_NAME);
        set_reg_sz(hkey, "DisplayIcon", &icon_str);
        set_reg_sz(hkey, "DisplayVersion", env!("CARGO_PKG_VERSION"));
        set_reg_sz(hkey, "Publisher", "hyak");
        set_reg_sz(hkey, "InstallLocation", &install_dir.to_string_lossy());
        set_reg_sz(hkey, "UninstallString", &uninst_str);
        set_reg_dword(hkey, "NoModify", 1);
        set_reg_dword(hkey, "NoRepair", 1);
        set_reg_dword(hkey, "EstimatedSize", 495_000);

        let _ = RegCloseKey(hkey);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (install_dir, target_exe);
        Ok(())
    }
}

/// Keeps "Aplicativos instalados" showing the running version: a self-update swaps the executable
/// without going through the installer, so the registered `DisplayVersion` would stay behind.
pub fn sync_registered_version() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        };
        let Some(dir) = read_registered_install_dir() else {
            return;
        };
        let runs_from_install_dir = env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.to_string_lossy().to_lowercase()))
            .map(|parent| parent == dir.to_string_lossy().trim_end_matches('\\').to_lowercase())
            .unwrap_or(false);
        if !runs_from_install_dir {
            return;
        }
        let subkey = wide_null("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Boladão");
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_SET_VALUE, &mut hkey) == 0 {
            set_reg_sz(hkey, "DisplayVersion", env!("CARGO_PKG_VERSION"));
            let _ = RegCloseKey(hkey);
        }
    }
}

fn read_registered_install_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ,
        };
        let subkey = wide_null("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Boladão");
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return None;
        }
        let wname = wide_null("InstallLocation");
        let mut reg_type: u32 = 0;
        let mut buf = vec![0u16; 520];
        let mut byte_len = (buf.len() * 2) as u32;
        let ok = RegQueryValueExW(
            hkey,
            wname.as_ptr(),
            std::ptr::null(),
            &mut reg_type,
            buf.as_mut_ptr() as *mut u8,
            &mut byte_len,
        );
        let _ = RegCloseKey(hkey);
        if ok == 0 && reg_type == REG_SZ && byte_len >= 2 {
            let u16_len = (byte_len as usize / 2).saturating_sub(1);
            let s = String::from_utf16_lossy(&buf[..u16_len]);
            let trimmed = s.trim().trim_matches('"');
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn unregister_windows_uninstaller() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Registry::{RegDeleteTreeW, HKEY_CURRENT_USER};
        let subkey = wide_null("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Boladão");
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, subkey.as_ptr());
        let legacy_manu = wide_null("Software\\Boladão");
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, legacy_manu.as_ptr());
    }
}
