use crate::classify::MediaMode;
use crate::paths::{
    canonical_template, compile_ytdlp_output_template, compute_safe_trim_length,
    untitled_template, TEMP_SUBDIR_NAME,
};
use crate::spec::{
    AudioFormat, DownloadSpec, LiveDownloadMode, Mp3Quality, SponsorBlockAction, SubtitleMode,
    VideoProfile,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Forbidden flags that user-supplied `extra_ytdlp_args` may NEVER set (Section 7.4 & 15).
const FORBIDDEN_EXTRA_FLAGS: &[&str] = &[
    "--exec",
    "--netrc",
    "--netrc-cmd",
    "--netrc-location",
    "-o",
    "--output",
    "-P",
    "--paths",
    "--proxy",
    "--ffmpeg-location",
    "--js-runtimes",
    "-a",
    "--batch-file",
    "--config-locations",
    "--load-info-json",
    "--write-link",
    "--write-url-link",
    "--write-webloc-link",
    "--write-desktop-link",
    "--print-to-file",
    "--plugin-dirs",
];

#[derive(Debug, Clone)]
pub enum CookieSource {
    None,
    File(PathBuf),
    FirefoxBrowser,
}

#[derive(Debug, Clone)]
pub struct ExecContext {
    pub ffmpeg_dir: PathBuf,
    pub deno_exe: PathBuf,
    pub proxy_url: Option<String>,
    pub fallback_limit_bps: Option<u64>,
    pub cookies: CookieSource,
    pub archive_file: Option<PathBuf>,
    pub apply_anti_429_sleep: bool,
    pub impersonate_target: Option<String>,
    pub playlist_index_override: Option<u32>,
}

/// Validates user-supplied extra arguments against forbidden options.
pub fn validate_extra_args(extra: &[String]) -> Result<(), String> {
    for raw in extra {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let flag_name = trimmed.split('=').next().unwrap_or(trimmed);
        for &forbidden in FORBIDDEN_EXTRA_FLAGS {
            if flag_name.eq_ignore_ascii_case(forbidden)
                || (forbidden.len() == 2 && trimmed.starts_with(forbidden))
            {
                return Err(tr!(
                    "O argumento '{forbidden}' é controlado pelo Boladão e não pode ser usado nos extras.",
                    "The argument '{forbidden}' is managed by Boladão and can't be used in the extras.",
                    "El argumento '{forbidden}' lo controla Boladão y no se puede usar en los extras."
                ));
            }
        }
    }
    Ok(())
}

/// Pure function: builds the complete `Vec<OsString>` passed to `yt-dlp.exe` (never via shell).
pub fn build_ytdlp_args(
    url: &str,
    spec: &DownloadSpec,
    ctx: &ExecContext,
) -> Result<Vec<OsString>, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(tr!(
            "URL inválida: apenas http:// e https:// são permitidos.",
            "Invalid URL: only http:// and https:// are allowed.",
            "URL no válida: solo se permiten http:// y https://."
        ));
    }
    validate_extra_args(&spec.extra_ytdlp_args)?;

    let mut args: Vec<OsString> = Vec::with_capacity(48);
    let push = |v: &mut Vec<OsString>, s: &str| v.push(OsString::from(s));
    let push2 = |v: &mut Vec<OsString>, a: &str, b: &str| {
        v.push(OsString::from(a));
        v.push(OsString::from(b));
    };

    // Base deterministic & progress-telemetry flags (Section 7.1)
    push(&mut args, "--ignore-config");
    push2(&mut args, "--encoding", "utf-8");
    push2(&mut args, "--color", "never");
    push(&mut args, "--newline");
    push(&mut args, "--progress");
    push2(&mut args, "--progress-delta", "0.15");
    push2(
        &mut args,
        "--progress-template",
        "download:BOLADAO_DL %(progress)j",
    );
    push2(
        &mut args,
        "--progress-template",
        "postprocess:BOLADAO_PP %(progress)j",
    );
    push2(
        &mut args,
        "--print",
        "after_move:BOLADAO_FILE %(filepath)j",
    );

    // Embedded FFmpeg + Deno JS runtime
    args.push(OsString::from("--ffmpeg-location"));
    args.push(ctx.ffmpeg_dir.as_os_str().to_os_string());
    push2(
        &mut args,
        "--js-runtimes",
        &format!("deno:{}", ctx.deno_exe.display()),
    );

    // Bandwidth limiter local proxy (or fallback --limit-rate)
    if let Some(proxy) = &ctx.proxy_url {
        push2(&mut args, "--proxy", proxy);
        // Force native downloader on HLS/DASH so traffic stays inside our token-bucket proxy (Section 11.2)
        push2(&mut args, "--downloader", "dash,m3u8:native");
    } else if let Some(bps) = ctx.fallback_limit_bps {
        if bps > 0 {
            push2(&mut args, "--limit-rate", &bps.to_string());
        }
    }

    // Output directories (temp on same disk as destination!) & Windows sanitization
    let home_dir = &spec.output_root;
    let temp_dir = home_dir.join(TEMP_SUBDIR_NAME);
    push2(&mut args, "--paths", &format!("home:{}", home_dir.display()));
    push2(&mut args, "--paths", &format!("temp:{}", temp_dir.display()));

    // `{tipo}` becomes the folder of the media type, in the interface language.
    let kind_folder = match spec.mode {
        MediaMode::Audio => tr!("Música", "Music", "Música"),
        MediaMode::Video => tr!("Vídeos", "Videos", "Videos"),
    };
    let mut tpl = canonical_template(&spec.output_template).replace("{tipo}", &kind_folder);
    if spec.playlist_numbering && !tpl.contains("{nn}") {
        if let Some(idx) = tpl.rfind('\\').or_else(|| tpl.rfind('/')) {
            let (prefix, rest) = tpl.split_at(idx + 1);
            tpl = format!("{prefix}{{nn}} - {rest}");
        } else {
            tpl = format!("{{nn}} - {tpl}");
        }
    }
    // If expanded from a playlist into individual jobs, allow deterministic numbering via playlist_index_override
    let mut compiled_tpl = compile_ytdlp_output_template(&tpl);
    if let Some(idx) = ctx.playlist_index_override {
        compiled_tpl = compiled_tpl.replace("%(playlist_index|01)02d", &format!("{idx:02}"));
    }

    let trim_len = compute_safe_trim_length(home_dir, &tpl);
    push2(&mut args, "--output", &compiled_tpl);
    if spec.split_chapters {
        let chapter_tpl = compiled_tpl.replace(
            &untitled_template(),
            "%(section_number)02d - %(section_title)s",
        );
        push2(&mut args, "--output", &format!("chapter:{chapter_tpl}"));
    }
    push(&mut args, "--windows-filenames");
    push2(&mut args, "--trim-filenames", &trim_len.to_string());
    push(&mut args, "--no-mtime");

    // Playlists are expanded by Boladão into grouped jobs EXCEPT when concat_playlist is selected (Section 7.3)
    if spec.concat_playlist {
        push(&mut args, "--yes-playlist");
        push2(&mut args, "--concat-playlist", "always");
    } else {
        push(&mut args, "--no-playlist");
    }

    push2(&mut args, "--concurrent-fragments", "4");
    push2(&mut args, "--retries", "10");
    push2(&mut args, "--fragment-retries", "10");
    push2(&mut args, "--retry-sleep", "exp=1:20");

    // Large playlist anti-429 sleep preset (`-t sleep` verified in yt-dlp 2026.10.26)
    if ctx.apply_anti_429_sleep {
        push2(&mut args, "--sleep-subtitles", "5");
        push2(&mut args, "--sleep-requests", "0.75");
        push2(&mut args, "--sleep-interval", "10");
        push2(&mut args, "--max-sleep-interval", "20");
    }

    if let Some(target) = &ctx.impersonate_target {
        push2(&mut args, "--impersonate", target);
    }

    if spec.use_download_archive {
        if let Some(archive) = &ctx.archive_file {
            args.push(OsString::from("--download-archive"));
            args.push(archive.as_os_str().to_os_string());
        }
    }

    match &ctx.cookies {
        CookieSource::None => {}
        CookieSource::File(p) => {
            args.push(OsString::from("--cookies"));
            args.push(p.as_os_str().to_os_string());
        }
        CookieSource::FirefoxBrowser => {
            push2(&mut args, "--cookies-from-browser", "firefox");
        }
    }

    // Mode-specific flags (Sections 7.2 & 7.3)
    match spec.mode {
        MediaMode::Audio => {
            match spec.audio_format {
                AudioFormat::Original
                | AudioFormat::Flac
                | AudioFormat::Alac
                | AudioFormat::Wav
                | AudioFormat::Aiff => {
                    push2(&mut args, "-f", "ba/b");
                    push(&mut args, "-x");
                    push2(&mut args, "--audio-format", "best");
                }
                AudioFormat::Mp3 => {
                    push2(&mut args, "-f", "ba/b");
                    push(&mut args, "-x");
                    push2(&mut args, "--audio-format", "mp3");
                    match spec.mp3_quality {
                        Mp3Quality::V0 => push2(&mut args, "--audio-quality", "0"),
                        Mp3Quality::Cbr320 => push2(&mut args, "--audio-quality", "320K"),
                    }
                }
                AudioFormat::M4a => {
                    push2(&mut args, "-f", "ba[acodec^=mp4a]/ba/b");
                    push(&mut args, "-x");
                    push2(&mut args, "--audio-format", "m4a");
                }
                AudioFormat::Opus => {
                    push2(&mut args, "-f", "ba/b");
                    push(&mut args, "-x");
                    push2(&mut args, "--audio-format", "opus");
                }
            }

            if spec.embed_metadata {
                push(&mut args, "--embed-metadata");
            }
            if spec.parse_artist_title {
                push2(
                    &mut args,
                    "--parse-metadata",
                    "title:%(artist)s - %(track)s",
                );
            }
            if spec.clean_music_title {
                push2(
                    &mut args,
                    "--replace-in-metadata",
                    "title,track",
                    // Remove common video noise suffixes
                );
                // Add regex pattern and empty replacement for --replace-in-metadata (takes 3 args: FIELDS REGEX REPLACE)
                push(
                    &mut args,
                    r"(?i)\s*[\(\[](official\s*(music\s*)?video|lyric\s*video|visualizer|official\s*audio|clipe\s*oficial|hd|4k)[\)\]]",
                );
                push(&mut args, "");
            }

            // Square 1:1 album cover art using comma-free algebraic min(iw,ih) = (iw+ih-abs(iw-ih))/2
            // so Python shlex.split inside yt-dlp `--ppa` and FFmpeg filtergraph never clash on commas/quotes!
            if spec.embed_square_cover && spec.audio_format.supports_square_cover() {
                const SQUARE_CROP: &str = "crop=(iw+ih-abs(iw-ih))/2:(iw+ih-abs(iw-ih))/2";
                if spec.audio_format.uses_workshop_conversion() {
                    // Save square-cropped png alongside temp so Workshop embeds 1:1 art into FLAC/ALAC even if source was .jpg!
                    push(&mut args, "--write-thumbnail");
                    push2(&mut args, "--convert-thumbnails", "png");
                    push2(
                        &mut args,
                        "--ppa",
                        &format!("ThumbnailsConvertor+ffmpeg_o:-vf {SQUARE_CROP}"),
                    );
                } else {
                    push(&mut args, "--embed-thumbnail");
                    // Converting to png forces ThumbnailsConvertor to run even when YouTube serves a 16:9 .jpg!
                    push2(&mut args, "--convert-thumbnails", "png");
                    push2(
                        &mut args,
                        "--ppa",
                        &format!("ThumbnailsConvertor+ffmpeg_o:-vf {SQUARE_CROP}"),
                    );
                    push2(
                        &mut args,
                        "--ppa",
                        &format!("EmbedThumbnail+ffmpeg_o:-c:v mjpeg -q:v 2 -vf {SQUARE_CROP}"),
                    );
                }
            }
        }
        MediaMode::Video => {
            let mut sort_parts: Vec<String> = Vec::new();
            if let Some(h) = spec.max_height {
                sort_parts.push(format!("res:{h}"));
            }
            if let Some(lang) = &spec.audio_language {
                if !lang.is_empty() && lang != "original" {
                    sort_parts.push(format!("lang:{lang}"));
                }
            }

            match spec.video_profile {
                VideoProfile::Compatible => {
                    push2(&mut args, "-f", "bv*+ba/b");
                    sort_parts.push("vcodec:h264".to_string());
                    sort_parts.push("acodec:aac".to_string());
                    push2(&mut args, "-S", &sort_parts.join(","));
                    push2(&mut args, "--merge-output-format", "mp4");
                    push2(&mut args, "--remux-video", "mp4");
                }
                VideoProfile::MaxQuality => {
                    push2(&mut args, "-f", "bv*+ba/b");
                    if !sort_parts.is_empty() {
                        push2(&mut args, "-S", &sort_parts.join(","));
                    }
                    push2(&mut args, "--merge-output-format", "mkv");
                }
                VideoProfile::Custom => {
                    push2(&mut args, "-f", "bv*+ba/b");
                    if let Some(vc) = &spec.custom_vcodec {
                        if !vc.is_empty() {
                            sort_parts.push(format!("vcodec:{vc}"));
                        }
                    }
                    if !sort_parts.is_empty() {
                        push2(&mut args, "-S", &sort_parts.join(","));
                    }
                    push2(
                        &mut args,
                        "--merge-output-format",
                        spec.custom_container.as_str(),
                    );
                }
            }

            if spec.embed_metadata {
                push(&mut args, "--embed-metadata");
            }
            if spec.embed_chapters {
                push(&mut args, "--embed-chapters");
            }

            match spec.subtitle_mode {
                SubtitleMode::Off => {}
                SubtitleMode::Embed => {
                    push(&mut args, "--write-subs");
                    if spec.include_auto_subs {
                        push(&mut args, "--write-auto-subs");
                    }
                    if !spec.subtitle_langs.is_empty() {
                        push2(&mut args, "--sub-langs", &spec.subtitle_langs.join(","));
                    }
                    push(&mut args, "--embed-subs");
                }
                SubtitleMode::SidecarSrt => {
                    push(&mut args, "--write-subs");
                    if spec.include_auto_subs {
                        push(&mut args, "--write-auto-subs");
                    }
                    if !spec.subtitle_langs.is_empty() {
                        push2(&mut args, "--sub-langs", &spec.subtitle_langs.join(","));
                    }
                    push2(&mut args, "--convert-subs", "srt");
                }
            }

            // If burn_subtitles is enabled in Workshop and subtitles weren't already written, write sidecar srt
            if spec.workshop.burn_subtitles && spec.subtitle_mode == SubtitleMode::Off {
                push(&mut args, "--write-subs");
                if spec.include_auto_subs {
                    push(&mut args, "--write-auto-subs");
                }
                if !spec.subtitle_langs.is_empty() {
                    push2(&mut args, "--sub-langs", &spec.subtitle_langs.join(","));
                }
                push2(&mut args, "--convert-subs", "srt");
            }
        }
    }

    // SponsorBlock (works for both Audio e.g. music_offtopic and Video)
    match spec.sponsorblock_action {
        SponsorBlockAction::Off => {}
        SponsorBlockAction::Remove => {
            if !spec.sponsorblock_categories.is_empty() {
                push2(
                    &mut args,
                    "--sponsorblock-remove",
                    &spec.sponsorblock_categories.join(","),
                );
            }
        }
        SponsorBlockAction::Mark => {
            if !spec.sponsorblock_categories.is_empty() {
                push2(
                    &mut args,
                    "--sponsorblock-mark",
                    &spec.sponsorblock_categories.join(","),
                );
            }
        }
    }

    // Section trimming (`--download-sections "*start-end"`)
    if spec.section_start_s.is_some() || spec.section_end_s.is_some() {
        let start = spec
            .section_start_s
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "0".to_string());
        let end = spec
            .section_end_s
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "inf".to_string());
        push2(&mut args, "--download-sections", &format!("*{start}-{end}"));
        if spec.force_keyframes_at_cuts {
            push(&mut args, "--force-keyframes-at-cuts");
        }
    }

    if spec.split_chapters {
        push(&mut args, "--split-chapters");
    }

    match spec.live_mode {
        LiveDownloadMode::FromNow => {}
        LiveDownloadMode::FromStart => {
            push(&mut args, "--live-from-start");
        }
        LiveDownloadMode::WaitForPremiere => {
            push2(&mut args, "--wait-for-video", "15-120");
        }
    }

    for extra in &spec.extra_ytdlp_args {
        let trimmed = extra.trim();
        if !trimmed.is_empty() {
            push(&mut args, trimmed);
        }
    }

    // Separate options from URL with `--` so a weird URL never parses as an option
    push(&mut args, "--");
    push(&mut args, url);

    Ok(args)
}

/// Formats an executable + `OsString` slice as a readable, copy-pasteable Windows command line
/// (redacting session proxy credentials so copying to share doesn't leak local session tokens, or optional).
pub fn format_equivalent_command(exe: &Path, args: &[OsString], redact_proxy_auth: bool) -> String {
    let quote = |s: &str| -> String {
        if s.is_empty() || s.contains(|c: char| c.is_whitespace() || matches!(c, '"' | '&' | '|' | '(' | ')')) {
            format!("\"{}\"", s.replace('"', "\\\""))
        } else {
            s.to_string()
        }
    };

    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote(&exe.to_string_lossy()));

    let mut redact_next = false;
    for arg in args {
        let raw = arg.to_string_lossy();
        if redact_next && redact_proxy_auth {
            redact_next = false;
            if let Some(at_pos) = raw.find('@') {
                let redacted = format!("http://<sessao>{}", &raw[at_pos..]);
                parts.push(quote(&redacted));
                continue;
            }
        }
        if raw == "--proxy" {
            redact_next = true;
        }
        parts.push(quote(&raw));
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::builtin_presets;

    fn test_ctx() -> ExecContext {
        ExecContext {
            ffmpeg_dir: PathBuf::from(r"C:\App\bin\ffmpeg"),
            deno_exe: PathBuf::from(r"C:\App\bin\deno\deno.exe"),
            proxy_url: Some("http://u:p@127.0.0.1:54321".to_string()),
            fallback_limit_bps: None,
            cookies: CookieSource::None,
            archive_file: Some(PathBuf::from(r"C:\Music\Boladão\.archive.txt")),
            apply_anti_429_sleep: false,
            impersonate_target: None,
            playlist_index_override: None,
        }
    }

    #[test]
    fn snapshots_all_builtin_presets() {
        let music = PathBuf::from(r"C:\Users\leo\Music\Boladão");
        let video = PathBuf::from(r"C:\Users\leo\Videos\Boladão");
        let presets = builtin_presets(&music, &video);
        let ctx = test_ctx();

        let rendered: Vec<(String, Vec<String>)> = presets
            .into_iter()
            .map(|p| {
                let args = build_ytdlp_args("https://www.youtube.com/watch?v=dQw4w9WgXcQ", &p.spec, &ctx)
                    .unwrap()
                    .into_iter()
                    .map(|s| s.to_string_lossy().into_owned())
                    .collect();
                (p.id, args)
            })
            .collect();

        insta::assert_yaml_snapshot!("builtin_presets_args", rendered);
    }

    #[test]
    fn snapshots_rich_extras_combination() {
        let mut ctx = test_ctx();
        ctx.cookies = CookieSource::File(PathBuf::from(r"C:\App\cookies\youtube.txt"));
        ctx.apply_anti_429_sleep = true;

        let spec = DownloadSpec {
            mode: MediaMode::Video,
            video_profile: VideoProfile::MaxQuality,
            max_height: Some(2160),
            audio_language: Some("pt".to_string()),
            subtitle_mode: SubtitleMode::Embed,
            subtitle_langs: vec!["pt.*".to_string(), "en".to_string()],
            include_auto_subs: true,
            sponsorblock_action: SponsorBlockAction::Remove,
            sponsorblock_categories: vec!["sponsor".to_string(), "selfpromo".to_string()],
            section_start_s: Some(12.5),
            section_end_s: Some(95.0),
            force_keyframes_at_cuts: true,
            split_chapters: true,
            live_mode: LiveDownloadMode::FromStart,
            output_root: PathBuf::from(r"D:\Videos"),
            output_template: "{plataforma}\\{canal}\\{titulo}".to_string(),
            playlist_numbering: true,
            use_download_archive: true,
            ..Default::default()
        };

        let args: Vec<String> =
            build_ytdlp_args("https://www.youtube.com/watch?v=test4K", &spec, &ctx)
                .unwrap()
                .into_iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();

        insta::assert_yaml_snapshot!("rich_video_extras_args", args);
    }

    #[test]
    fn templates_accept_any_language_and_fill_the_media_type() {
        let ctx = test_ctx();
        let output_of = |template: &str, mode: MediaMode| -> String {
            let spec = DownloadSpec {
                mode,
                output_root: PathBuf::from(r"D:\Media"),
                output_template: template.to_string(),
                ..Default::default()
            };
            let args: Vec<String> = build_ytdlp_args("https://www.youtube.com/watch?v=x", &spec, &ctx)
                .unwrap()
                .into_iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            let i = args.iter().position(|a| a == "--output").unwrap();
            args[i + 1].clone()
        };

        let original = output_of("{tipo}\\{artista}\\{faixa}", MediaMode::Audio);
        assert_eq!(output_of("{type}\\{artist}\\{track}", MediaMode::Audio), original);
        assert_eq!(output_of("{tipo}\\{artista}\\{pista}", MediaMode::Audio), original);
        // `{tipo}` is the media folder itself, not a yt-dlp field that never exists.
        assert!(original.starts_with("Música\\"), "{original}");
        assert!(output_of("{tipo}\\{titulo}", MediaMode::Video).starts_with("Vídeos\\"));
    }

    #[test]
    fn blocks_dangerous_extra_flags() {
        for bad in [
            "--exec calc.exe",
            "--exec=calc.exe",
            "-o C:\\evil.exe",
            "--proxy=http://evil",
            "--netrc-cmd",
            "--load-info-json",
        ] {
            let first_tok = bad.split_whitespace().next().unwrap().to_string();
            assert!(
                validate_extra_args(&[first_tok]).is_err(),
                "Should reject {bad}"
            );
        }
    }
}
