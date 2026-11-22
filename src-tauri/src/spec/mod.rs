use crate::classify::MediaMode;
use crate::paths::default_video_root;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Original,
    Mp3,
    M4a,
    Opus,
    Flac,
    Alac,
    Wav,
    Aiff,
}

impl AudioFormat {
    /// Formats converted in our FFmpeg Workshop after extracting Original lossless stream.
    pub fn uses_workshop_conversion(self) -> bool {
        matches!(self, Self::Flac | Self::Alac | Self::Wav | Self::Aiff)
    }

    /// Formats that support square embedded cover art (WAV/AIFF do not reliably display art across players).
    pub fn supports_square_cover(self) -> bool {
        matches!(self, Self::Mp3 | Self::M4a | Self::Opus | Self::Flac | Self::Alac)
    }

    pub fn extension_hint(self) -> &'static str {
        match self {
            Self::Original => "opus",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
            Self::Opus => "opus",
            Self::Flac => "flac",
            Self::Alac => "m4a",
            Self::Wav => "wav",
            Self::Aiff => "aiff",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mp3Quality {
    V0,
    Cbr320,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WavBitDepth {
    S16,
    S24,
    F32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoProfile {
    Compatible,
    MaxQuality,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoContainer {
    Mp4,
    Mkv,
    Webm,
    Mov,
}

impl VideoContainer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
            Self::Mov => "mov",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SponsorBlockAction {
    Off,
    Remove,
    Mark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleMode {
    Off,
    Embed,
    SidecarSrt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveDownloadMode {
    FromNow,
    FromStart,
    WaitForPremiere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReencodeQuality {
    High,
    Medium,
    Small,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GifSpec {
    pub fps: u8,
    pub width: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WhisperSpec {
    pub model: String,    // "base" | "small"
    pub language: String, // "auto" | "pt" | "en" | ...
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VideoReencodeSpec {
    pub codec: String, // "h264" | "hevc" | "av1"
    pub quality: ReencodeQuality,
    pub prefer_gpu: bool,
}

impl Default for ReencodeQuality {
    fn default() -> Self {
        Self::High
    }
}

/// Optional FFmpeg post-processing pipeline executed after `yt-dlp` finishes.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkshopSpec {
    // Audio operations
    pub loudnorm_lufs: Option<f32>, // e.g. Some(-14.0), Some(-16.0), Some(-23.0)
    pub trim_silence: bool,
    pub fade_in_s: Option<f32>,
    pub fade_out_s: Option<f32>,
    pub speed_factor: Option<f32>,    // e.g. 1.25
    pub pitch_semitones: Option<f32>, // requires rubberband
    pub channels: Option<u8>,         // 1 (mono) | 2 (stereo)

    // Video operations
    pub video_reencode: Option<VideoReencodeSpec>,
    pub target_size_mb: Option<f32>,
    pub scale_height: Option<u32>,
    pub target_fps: Option<u32>,
    pub make_gif: Option<GifSpec>,
    pub extract_frame_at_s: Option<f64>,
    pub burn_subtitles: bool,

    // Experimental Whisper transcription
    pub whisper: Option<WhisperSpec>,
}

impl WorkshopSpec {
    pub fn has_any_step(&self, mode: MediaMode, audio_format: AudioFormat) -> bool {
        (mode == MediaMode::Audio && audio_format.uses_workshop_conversion())
            || self.loudnorm_lufs.is_some()
            || self.trim_silence
            || self.fade_in_s.filter(|&v| v > 0.0).is_some()
            || self.fade_out_s.filter(|&v| v > 0.0).is_some()
            || self.speed_factor.filter(|&v| (v - 1.0).abs() > 0.01).is_some()
            || self.pitch_semitones.filter(|&v| v.abs() > 0.01).is_some()
            || self.channels.is_some()
            || self.video_reencode.is_some()
            || self.target_size_mb.is_some()
            || self.scale_height.is_some()
            || self.target_fps.is_some()
            || self.make_gif.is_some()
            || self.extract_frame_at_s.is_some()
            || self.burn_subtitles
            || self.whisper.is_some()
    }

    pub fn is_heavy(&self) -> bool {
        self.video_reencode.is_some()
            || self.target_size_mb.is_some()
            || self.burn_subtitles
            || self.whisper.is_some()
            || self.make_gif.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSpec {
    pub mode: MediaMode,

    // Audio
    pub audio_format: AudioFormat,
    pub mp3_quality: Mp3Quality,
    pub wav_bit_depth: WavBitDepth,
    pub sample_rate_hz: Option<u32>, // e.g. 44100, 48000, 96000
    pub embed_square_cover: bool,
    pub embed_metadata: bool,
    pub parse_artist_title: bool,
    pub clean_music_title: bool,

    // Video
    pub video_profile: VideoProfile,
    pub max_height: Option<u32>, // None = unlimited / best
    pub custom_container: VideoContainer,
    pub custom_vcodec: Option<String>, // "h264" | "vp9" | "av01" | "hevc"
    pub audio_language: Option<String>,
    pub subtitle_mode: SubtitleMode,
    pub subtitle_langs: Vec<String>,
    pub include_auto_subs: bool,
    pub embed_chapters: bool,

    // SponsorBlock & sections
    pub sponsorblock_action: SponsorBlockAction,
    pub sponsorblock_categories: Vec<String>,
    pub section_start_s: Option<f64>,
    pub section_end_s: Option<f64>,
    pub force_keyframes_at_cuts: bool,
    pub split_chapters: bool,
    pub concat_playlist: bool,
    pub live_mode: LiveDownloadMode,

    // Output & naming
    pub output_root: PathBuf,
    pub output_template: String,
    pub playlist_numbering: bool,
    pub use_download_archive: bool,

    // Workshop & advanced
    pub workshop: WorkshopSpec,
    pub extra_ytdlp_args: Vec<String>,
}

impl Default for DownloadSpec {
    fn default() -> Self {
        Self {
            mode: MediaMode::Video,
            audio_format: AudioFormat::Original,
            mp3_quality: Mp3Quality::V0,
            wav_bit_depth: WavBitDepth::S24,
            sample_rate_hz: Some(48_000),
            embed_square_cover: true,
            embed_metadata: true,
            parse_artist_title: true,
            clean_music_title: true,
            video_profile: VideoProfile::Compatible,
            max_height: Some(1080),
            custom_container: VideoContainer::Mp4,
            custom_vcodec: None,
            audio_language: None,
            subtitle_mode: SubtitleMode::Off,
            subtitle_langs: vec!["pt.*".to_string(), "en.*".to_string()],
            include_auto_subs: false,
            embed_chapters: true,
            sponsorblock_action: SponsorBlockAction::Off,
            sponsorblock_categories: vec!["sponsor".to_string()],
            section_start_s: None,
            section_end_s: None,
            force_keyframes_at_cuts: false,
            split_chapters: false,
            concat_playlist: false,
            live_mode: LiveDownloadMode::FromNow,
            output_root: default_video_root(),
            output_template: "{plataforma}\\{canal}\\{titulo} [{id}]".to_string(),
            playlist_numbering: false,
            use_download_archive: false,
            workshop: WorkshopSpec::default(),
            extra_ytdlp_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub builtin: bool,
    pub spec: DownloadSpec,
}

pub fn builtin_presets(music_root: &PathBuf, video_root: &PathBuf) -> Vec<Preset> {
    let audio_tpl = "{artista}\\{album}\\{faixa}".to_string();
    let video_tpl = "{plataforma}\\{canal}\\{titulo} [{id}]".to_string();

    vec![
        Preset {
            id: "audio-original".to_string(),
            name: tr!("Original (áudio)", "Original (audio)", "Original (audio)"),
            description: tr!("Mesma qualidade da fonte, sem reconversão (Opus/AAC).", "Same quality as the source, no re-encoding (Opus/AAC).", "Misma calidad que la fuente, sin recodificar (Opus/AAC)."),
            builtin: true,
            spec: DownloadSpec {
                mode: MediaMode::Audio,
                audio_format: AudioFormat::Original,
                output_root: music_root.clone(),
                output_template: audio_tpl.clone(),
                ..Default::default()
            },
        },
        Preset {
            id: "audio-mp3-v0".to_string(),
            name: "MP3 V0".to_string(),
            description: tr!("MP3 VBR máxima qualidade com capa quadrada e metadados.", "Highest-quality VBR MP3 with a square cover and metadata.", "MP3 VBR de máxima calidad con portada cuadrada y metadatos."),
            builtin: true,
            spec: DownloadSpec {
                mode: MediaMode::Audio,
                audio_format: AudioFormat::Mp3,
                mp3_quality: Mp3Quality::V0,
                output_root: music_root.clone(),
                output_template: audio_tpl.clone(),
                ..Default::default()
            },
        },
        Preset {
            id: "audio-wav-24-48".to_string(),
            name: "WAV 24-bit / 48 kHz".to_string(),
            description: tr!("PCM 24-bit/48 kHz reamostrado via SoXR para DAWs e editores.", "24-bit/48 kHz PCM resampled with SoXR for DAWs and editors.", "PCM 24-bit/48 kHz remuestreado con SoXR para DAW y editores."),
            builtin: true,
            spec: DownloadSpec {
                mode: MediaMode::Audio,
                audio_format: AudioFormat::Wav,
                wav_bit_depth: WavBitDepth::S24,
                sample_rate_hz: Some(48_000),
                embed_square_cover: false,
                output_root: music_root.clone(),
                output_template: audio_tpl,
                ..Default::default()
            },
        },
        Preset {
            id: "video-compat-1080".to_string(),
            name: tr!("Vídeo compatível 1080p", "Compatible video 1080p", "Video compatible 1080p"),
            description: tr!("MP4 H.264 + AAC até 1080p. Abre no WhatsApp, Premiere e qualquer TV.", "MP4 H.264 + AAC up to 1080p. Opens in WhatsApp, Premiere and any TV.", "MP4 H.264 + AAC hasta 1080p. Se abre en WhatsApp, Premiere y cualquier TV."),
            builtin: true,
            spec: DownloadSpec {
                mode: MediaMode::Video,
                video_profile: VideoProfile::Compatible,
                max_height: Some(1080),
                output_root: video_root.clone(),
                output_template: video_tpl.clone(),
                ..Default::default()
            },
        },
        Preset {
            id: "video-max-mkv".to_string(),
            name: tr!("Vídeo máxima qualidade", "Maximum-quality video", "Video de máxima calidad"),
            description: tr!("MKV mantendo AV1/VP9 + Opus até 4K/8K HDR sem reconverter.", "MKV keeping AV1/VP9 + Opus up to 4K/8K HDR, no re-encoding.", "MKV que conserva AV1/VP9 + Opus hasta 4K/8K HDR sin recodificar."),
            builtin: true,
            spec: DownloadSpec {
                mode: MediaMode::Video,
                video_profile: VideoProfile::MaxQuality,
                max_height: None,
                output_root: video_root.clone(),
                output_template: video_tpl.clone(),
                ..Default::default()
            },
        },
        Preset {
            id: "video-reels-tiktok".to_string(),
            name: "Reels/TikTok (MP4)".to_string(),
            description: tr!("MP4 pronto pra compartilhar rápido.", "MP4 ready to share quickly.", "MP4 listo para compartir rápido."),
            builtin: true,
            spec: DownloadSpec {
                mode: MediaMode::Video,
                video_profile: VideoProfile::Compatible,
                max_height: Some(1080),
                output_root: video_root.clone(),
                output_template: "{plataforma}\\{canal}\\{titulo} [{id}]".to_string(),
                ..Default::default()
            },
        },
        Preset {
            id: "video-clip-section".to_string(),
            name: tr!("Só o trecho", "Just the section", "Solo el fragmento"),
            description: tr!("Baixa apenas o intervalo marcado com corte preciso nos keyframes.", "Downloads only the marked range, cut precisely on keyframes.", "Descarga solo el intervalo marcado, con corte preciso en los fotogramas clave."),
            builtin: true,
            spec: DownloadSpec {
                mode: MediaMode::Video,
                video_profile: VideoProfile::Compatible,
                max_height: Some(1080),
                section_start_s: Some(0.0),
                section_end_s: Some(30.0),
                force_keyframes_at_cuts: true,
                output_root: video_root.clone(),
                output_template: video_tpl,
                ..Default::default()
            },
        },
    ]
}
