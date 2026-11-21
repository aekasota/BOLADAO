use crate::cookies::ConnectedAccount;
use crate::gamemode::GameModeOverride;
use crate::limiter::BandwidthProfile;
use crate::paths::{
    app_config_dir, default_music_root, default_video_root, detect_legacy_yt2_music_folder,
    CollisionPolicy, PlatformPathRule,
};
use crate::spec::{DownloadSpec, Preset};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeId {
    Sangue,
    Papel,
    Acido,
    Cobalto,
}

impl Default for ThemeId {
    fn default() -> Self {
        Self::Sangue
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeedUnit {
    BytesPerSec,
    BitsPerSec,
}

impl Default for SpeedUnit {
    fn default() -> Self {
        Self::BytesPerSec
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    // Folders & Organization
    pub music_root: PathBuf,
    pub video_root: PathBuf,
    pub audio_template: String,
    pub video_template: String,
    pub playlist_audio_template: String,
    pub playlist_video_template: String,
    pub platform_rules: Vec<PlatformPathRule>,
    pub recent_folders: Vec<PathBuf>,
    pub favorite_folders: Vec<PathBuf>,
    pub collision_policy: CollisionPolicy,
    pub migrated_from_yt2: Option<PathBuf>,
    pub migration_banner_dismissed: bool,

    // Network & Queue
    pub global_concurrency: usize,
    pub host_concurrency: BTreeMap<String, usize>,
    pub bandwidth_profile: BandwidthProfile,
    pub custom_limit_bps: u64,
    pub light_limit_bps: u64,
    pub peak_7d_bps: u64,
    pub speed_unit: SpeedUnit,
    pub monitor_clipboard_background: bool,
    pub tray_notice_shown: bool,

    // Game Mode
    pub game_mode_auto: bool,
    pub game_mode_override: GameModeOverride,
    pub game_exe_list: Vec<String>,
    pub game_bandwidth_profile: BandwidthProfile,
    pub game_custom_limit_bps: u64,
    pub game_concurrency: usize,
    pub game_pause_workshop: bool,

    // Accounts
    pub accounts: BTreeMap<String, ConnectedAccount>,

    // Defaults & Presets
    pub custom_presets: Vec<Preset>,
    pub last_spec_by_platform: BTreeMap<String, DownloadSpec>,

    // Appearance
    pub theme: ThemeId,
    #[serde(default = "default_shader_intensity")]
    pub shader_intensity: f32,
    pub shader_outside_intensity: f32,
    pub shader_stripe_density: f32,
    pub reduce_motion: bool,

    // Components & Advanced
    pub ytdlp_channel: String, // "stable" | "nightly"
    pub last_ytdlp_check: Option<DateTime<Utc>>,
    pub global_extra_ytdlp_args: Vec<String>,
    pub log_level: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        let mut host_conc = BTreeMap::new();
        host_conc.insert("youtube".to_string(), 2);
        host_conc.insert("youtube_music".to_string(), 2);
        host_conc.insert("instagram".to_string(), 1);
        host_conc.insert("tiktok".to_string(), 1);

        Self {
            music_root: default_music_root(),
            video_root: default_video_root(),
            audio_template: "{artista}\\{album}\\{faixa}".to_string(),
            video_template: "{plataforma}\\{canal}\\{titulo} [{id}]".to_string(),
            playlist_audio_template: "{artista}\\{playlist}\\{nn} - {faixa}".to_string(),
            playlist_video_template: "{plataforma}\\{canal}\\{playlist}\\{nn} - {titulo}".to_string(),
            platform_rules: vec![PlatformPathRule {
                platform: "instagram".to_string(),
                enabled: false,
                custom_root: None,
                template: "Instagram\\{canal}\\{titulo} [{id}]".to_string(),
            }],
            recent_folders: Vec::new(),
            favorite_folders: Vec::new(),
            collision_policy: CollisionPolicy::KeepBoth,
            migrated_from_yt2: None,
            migration_banner_dismissed: false,

            global_concurrency: 3,
            host_concurrency: host_conc,
            bandwidth_profile: BandwidthProfile::Unlimited,
            custom_limit_bps: 5 * 1024 * 1024,
            light_limit_bps: 2 * 1024 * 1024,
            peak_7d_bps: 0,
            speed_unit: SpeedUnit::BytesPerSec,
            monitor_clipboard_background: false,
            tray_notice_shown: false,

            game_mode_auto: true,
            game_mode_override: GameModeOverride::Auto,
            game_exe_list: vec![
                "cs2.exe".to_string(),
                "valorant-win64-shipping.exe".to_string(),
                "league of legends.exe".to_string(),
                "dota2.exe".to_string(),
                "eldenring.exe".to_string(),
            ],
            game_bandwidth_profile: BandwidthProfile::Light,
            game_custom_limit_bps: 2 * 1024 * 1024,
            game_concurrency: 1,
            game_pause_workshop: false,

            accounts: BTreeMap::new(),
            custom_presets: Vec::new(),
            last_spec_by_platform: BTreeMap::new(),

            theme: ThemeId::Sangue,
            shader_intensity: 1.0,
            shader_outside_intensity: 1.0,
            shader_stripe_density: 1.0,
            reduce_motion: false,

            ytdlp_channel: "stable".to_string(),
            last_ytdlp_check: None,
            global_extra_ytdlp_args: Vec::new(),
            log_level: "info".to_string(),
        }
    }
}

pub fn settings_path() -> PathBuf {
    app_config_dir().join("settings.json")
}

fn default_shader_intensity() -> f32 {
    1.0
}

pub fn load_or_init_settings() -> AppSettings {
    let path = settings_path();
    if path.is_file() {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(mut parsed) = serde_json::from_str::<AppSettings>(&raw) {
                parsed.global_concurrency = parsed.global_concurrency.clamp(1, 6);
                return parsed;
            }
        }
    }

    let mut fresh = AppSettings::default();
    if let Some((legacy_music, _src_cfg)) = detect_legacy_yt2_music_folder() {
        fresh.music_root = legacy_music.clone();
        fresh.migrated_from_yt2 = Some(legacy_music);
    }
    let _ = save_settings_to_disk(&fresh);
    fresh
}

pub fn save_settings_to_disk(settings: &AppSettings) -> Result<(), String> {
    let path = settings_path();
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(&tmp, json).map_err(|e| format!("Erro salvando settings.json: {e}"))?;
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
    fs::rename(&tmp, &path).map_err(|e| format!("Erro confirmando settings.json: {e}"))
}

#[cfg(test)]
mod wcag_tests {
    /// Parses `#RRGGBB` and computes WCAG 2.1 relative luminance.
    fn luminance(hex: &str) -> f64 {
        let h = hex.trim_start_matches('#');
        let r = u8::from_str_radix(&h[0..2], 16).unwrap() as f64 / 255.0;
        let g = u8::from_str_radix(&h[2..4], 16).unwrap() as f64 / 255.0;
        let b = u8::from_str_radix(&h[4..6], 16).unwrap() as f64 / 255.0;
        let ch = |c: f64| {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
    }

    fn contrast_ratio(fg: &str, bg: &str) -> f64 {
        let l1 = luminance(fg);
        let l2 = luminance(bg);
        let (bright, dark) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (bright + 0.05) / (dark + 0.05)
    }

    #[derive(Debug)]
    struct ThemePalette {
        name: &'static str,
        bg: &'static str,
        surface_1: &'static str,
        surface_2: &'static str,
        text: &'static str,
        text_2: &'static str,
        text_3: &'static str,
        accent_btn: &'static str,
        on_accent: &'static str,
        accent_text: &'static str,
        warn: &'static str,
    }

    /// Verifies Section 13.2 & Section 18.1: ALL text/background pairs across ALL 4 themes
    /// (`Sangue`, `Papel`, `Ácido`, `Cobalto`) achieve WCAG AA contrast >= 4.5:1!
    #[test]
    fn all_themes_satisfy_wcag_aa_4_5_contrast() {
        let themes = [
            ThemePalette {
                name: "Sangue",
                bg: "#0E0E10",
                surface_1: "#17171A",
                surface_2: "#1F1F23",
                text: "#F2EEE6",
                text_2: "#A8A39C",
                text_3: "#948F89",
                accent_btn: "#FA023E",
                on_accent: "#0E0E10",
                accent_text: "#FF5C7A",
                warn: "#FFB547",
            },
            ThemePalette {
                name: "Papel",
                bg: "#F2EEE6",
                surface_1: "#EAE4D8",
                surface_2: "#DFD8CA",
                text: "#161616",
                text_2: "#3D3A36",
                text_3: "#54504A",
                accent_btn: "#C80032",
                on_accent: "#F9F6F0",
                accent_text: "#B0002B",
                warn: "#8A4B00",
            },
            ThemePalette {
                name: "Ácido",
                bg: "#0D0E0B",
                surface_1: "#151712",
                surface_2: "#1D2019",
                text: "#F2F4EA",
                text_2: "#ABB0A0",
                text_3: "#919786",
                accent_btn: "#C6FF00",
                on_accent: "#121212",
                accent_text: "#C6FF00",
                warn: "#FFB547",
            },
            ThemePalette {
                name: "Cobalto",
                bg: "#0B0D16",
                surface_1: "#121624",
                surface_2: "#1A2033",
                text: "#EDF0FA",
                text_2: "#A3ACC4",
                text_3: "#8B95B0",
                accent_btn: "#5B78FF",
                on_accent: "#0B0D16",
                accent_text: "#7B93FF",
                warn: "#FFB547",
            },
        ];

        for t in themes {
            for (bg_label, bg) in [
                ("bg", t.bg),
                ("surface-1", t.surface_1),
                ("surface-2", t.surface_2),
            ] {
                for (fg_label, fg) in [
                    ("text", t.text),
                    ("text-2", t.text_2),
                    ("text-3", t.text_3),
                    ("accent-text", t.accent_text),
                    ("warn", t.warn),
                ] {
                    let ratio = contrast_ratio(fg, bg);
                    assert!(
                        ratio >= 4.5,
                        "Tema {} falhou WCAG AA em {fg_label} ({fg}) sobre {bg_label} ({bg}): {ratio:.2}:1 < 4.5:1",
                        t.name
                    );
                }
            }
            let btn_ratio = contrast_ratio(t.on_accent, t.accent_btn);
            assert!(
                btn_ratio >= 4.5,
                "Tema {} falhou WCAG AA no botão primário ({} sobre {}): {btn_ratio:.2}:1 < 4.5:1",
                t.name,
                t.on_accent,
                t.accent_btn
            );
        }
    }
}
