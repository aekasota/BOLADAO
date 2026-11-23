use crate::bins::{BinManager, FfmpegCapabilities};
use crate::classify::MediaMode;
use crate::paths::app_local_dir;
use crate::spec::{
    AudioFormat, DownloadSpec, Mp3Quality, ReencodeQuality, WavBitDepth, WorkshopSpec,
};
use crate::winproc::{run_hidden_output, spawn_in_job, JobGroup, ProcessPriority};
use futures_util::StreamExt;
use serde::Deserialize;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};

pub fn whisper_models_dir() -> PathBuf {
    let dir = app_local_dir().join("models");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Escapes a Windows path for use inside an FFmpeg filtergraph (`C:\foo\bar.srt` -> `C\:/foo/bar.srt`).
pub fn escape_ffmpeg_filter_path(p: &Path) -> String {
    p.to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

/// Probes duration in seconds of a local media file via `ffprobe`.
pub async fn probe_duration_seconds(ffprobe_exe: &Path, input: &Path) -> Option<f64> {
    let args = vec![
        OsString::from("-v"),
        OsString::from("error"),
        OsString::from("-show_entries"),
        OsString::from("format=duration"),
        OsString::from("-of"),
        OsString::from("default=noprint_wrappers=1:nokey=1"),
        input.as_os_str().to_os_string(),
    ];
    let (code, stdout, _) = run_hidden_output(ffprobe_exe, &args, ProcessPriority::BelowNormal)
        .await
        .ok()?;
    if code != 0 {
        return None;
    }
    stdout.trim().parse::<f64>().ok()
}

/// Runs an FFmpeg command inside the job's `JobGroup` and parses `-progress pipe:1` (`out_time_us`)
/// to report real `0.0 ..= 1.0` stage progress.
pub async fn run_ffmpeg_with_progress<F>(
    job_group: &JobGroup,
    ffmpeg_exe: &Path,
    args_without_progress: &[OsString],
    duration_s: Option<f64>,
    priority: ProcessPriority,
    mut on_progress: F,
) -> Result<String, String>
where
    F: FnMut(f64),
{
    let mut full_args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-y"),
        OsString::from("-nostats"),
        OsString::from("-progress"),
        OsString::from("pipe:1"),
    ];
    full_args.extend_from_slice(args_without_progress);

    let mut child = spawn_in_job(job_group, ffmpeg_exe, &full_args, priority, None)
        .map_err(|e| tr!("Falha iniciando ffmpeg: {e}", "Couldn't start ffmpeg: {e}", "No se pudo iniciar ffmpeg: {e}"))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let stderr_handle = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut collected = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            collected.push(line);
            if collected.len() > 200 {
                collected.remove(0);
            }
        }
        collected.join("\n")
    });

    let mut out_lines = BufReader::new(stdout).lines();
    let total_us = duration_s.filter(|&d| d > 0.1).map(|d| d * 1_000_000.0);

    while let Ok(Some(line)) = out_lines.next_line().await {
        if let Some(val) = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")) {
            if let (Some(tot), Ok(cur)) = (total_us, val.trim().parse::<f64>()) {
                if cur >= 0.0 {
                    on_progress((cur / tot).clamp(0.0, 0.99));
                }
            }
        } else if line.trim() == "progress=end" {
            on_progress(1.0);
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| tr!("Erro aguardando ffmpeg: {e}", "Error waiting for ffmpeg: {e}", "Error al esperar a ffmpeg: {e}"))?;
    let stderr_text = stderr_handle.await.unwrap_or_default();

    if !status.success() {
        return Err(format!(
            "FFmpeg retornou erro ({}): {stderr_text}",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(stderr_text)
}

/// Builds chained `atempo` filters for arbitrary speed factors (`atempo` supports `0.5..=2.0` per stage).
fn build_atempo_chain(mut factor: f32) -> Vec<String> {
    let mut stages = Vec::new();
    while factor > 2.0 {
        stages.push("atempo=2.0".to_string());
        factor /= 2.0;
    }
    while factor < 0.5 {
        stages.push("atempo=0.5".to_string());
        factor /= 0.5;
    }
    if (factor - 1.0).abs() > 0.005 {
        stages.push(format!("atempo={factor:.4}"));
    }
    stages
}

#[derive(Debug, Deserialize)]
struct LoudnormMeasurement {
    input_i: String,
    input_tp: String,
    input_lra: String,
    input_thresh: String,
    target_offset: String,
}

async fn measure_loudnorm_first_pass(
    job_group: &JobGroup,
    ffmpeg_exe: &Path,
    input: &Path,
    target_lufs: f32,
    priority: ProcessPriority,
) -> Result<LoudnormMeasurement, String> {
    let filter = format!("loudnorm=I={target_lufs:.1}:TP=-1.0:LRA=11:print_format=json");
    let args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostats"),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-vn"),
        OsString::from("-af"),
        OsString::from(filter),
        OsString::from("-f"),
        OsString::from("null"),
        OsString::from("-"),
    ];
    let child = spawn_in_job(job_group, ffmpeg_exe, &args, priority, None)
        .map_err(|e| tr!("Falha medindo loudnorm: {e}", "Couldn't measure loudnorm: {e}", "No se pudo medir loudnorm: {e}"))?;
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| tr!("Erro no loudnorm 1ª passada: {e}", "Error in the loudnorm 1st pass: {e}", "Error en la 1.ª pasada de loudnorm: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Extract the JSON object at the end of loudnorm output
    let start = stderr
        .rfind('{')
        .ok_or_else(|| tr!("JSON de medição loudnorm não encontrado.", "loudnorm measurement JSON not found.", "No se encontró el JSON de medición de loudnorm."))?;
    let end = stderr
        .rfind('}')
        .ok_or_else(|| tr!("JSON de medição loudnorm incompleto.", "Incomplete loudnorm measurement JSON.", "JSON de medición de loudnorm incompleto."))?;
    serde_json::from_str::<LoudnormMeasurement>(&stderr[start..=end])
        .map_err(|e| tr!("Falha lendo medição loudnorm: {e}", "Couldn't read the loudnorm measurement: {e}", "No se pudo leer la medición de loudnorm: {e}"))
}

/// Ensures the requested Whisper GGML model (`base` or `small`) exists locally, downloading on demand with progress.
async fn ensure_whisper_model<F>(
    model_name: &str,
    mut on_progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(&str, f64),
{
    let clean = match model_name {
        "small" => "small",
        _ => "base",
    };
    let filename = format!("ggml-{clean}.bin");
    let dest = whisper_models_dir().join(&filename);
    if dest.is_file() && fs::metadata(&dest).map(|m| m.len() > 10_000_000).unwrap_or(false) {
        return Ok(dest);
    }

    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{filename}?download=true"
    );
    let tmp = dest.with_extension("bin.part");
    on_progress(&tr!("Baixando modelo Whisper ({clean})...", "Downloading the Whisper model ({clean})...", "Descargando el modelo Whisper ({clean})..."), 0.0);

    let client = reqwest::Client::builder()
        .user_agent("Boladao-Desktop/1.0")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| tr!("Erro baixando modelo Whisper: {e}", "Error downloading the Whisper model: {e}", "Error al descargar el modelo Whisper: {e}"))?
        .error_for_status()
        .map_err(|e| tr!("Servidor de modelos retornou erro: {e}", "The model server returned an error: {e}", "El servidor de modelos devolvió un error: {e}"))?;

    let total = resp.content_length().unwrap_or(148 * 1024 * 1024);
    let mut file = fs::File::create(&tmp).map_err(|e| tr!("Erro criando {filename}: {e}", "Error creating {filename}: {e}", "Error al crear {filename}: {e}"))?;
    let mut stream = resp.bytes_stream();
    let mut downloaded = 0u64;

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| tr!("Download do modelo interrompido: {e}", "Model download interrupted: {e}", "Descarga del modelo interrumpida: {e}"))?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        on_progress(
            &tr!("Baixando modelo Whisper ({clean})...", "Downloading the Whisper model ({clean})...", "Descargando el modelo Whisper ({clean})..."),
            (downloaded as f64 / total as f64).clamp(0.0, 0.99),
        );
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&tmp, &dest).map_err(|e| tr!("Erro salvando modelo Whisper: {e}", "Error saving the Whisper model: {e}", "Error al guardar el modelo Whisper: {e}"))?;
    Ok(dest)
}

/// Chooses best available video encoder (GPU first when `prefer_gpu` is true, verified by 0.1s boot test, else CPU).
fn select_video_encoder<'a>(
    codec_family: &str,
    prefer_gpu: bool,
    caps: &'a FfmpegCapabilities,
) -> &'a str {
    if prefer_gpu {
        let candidates: &[&str] = match codec_family {
            "hevc" | "h265" => &["hevc_videotoolbox", "hevc_nvenc", "hevc_qsv", "hevc_amf"],
            "av1" => &["av1_nvenc", "av1_qsv", "av1_amf"],
            _ => &["h264_videotoolbox", "h264_nvenc", "h264_qsv", "h264_amf"],
        };
        for &cand in candidates {
            if caps.hw_encoders.iter().any(|h| h == cand) {
                return cand;
            }
        }
    }
    match codec_family {
        "hevc" | "h265" => "libx265",
        "av1" => "libsvtav1",
        _ => "libx264",
    }
}

fn append_quality_flags(args: &mut Vec<OsString>, encoder: &str, q: ReencodeQuality) {
    let (cq_or_crf, preset) = match q {
        ReencodeQuality::High => ("19", "slow"),
        ReencodeQuality::Medium => ("23", "medium"),
        ReencodeQuality::Small => ("28", "fast"),
    };
    if encoder.ends_with("_videotoolbox") {
        // VideoToolbox has no constant-quality mode on Intel Macs: a target bitrate works on all.
        let bitrate = match q {
            ReencodeQuality::High => "12M",
            ReencodeQuality::Medium => "7M",
            ReencodeQuality::Small => "3M",
        };
        args.push(OsString::from("-b:v"));
        args.push(OsString::from(bitrate));
    } else if encoder.contains("nvenc") {
        args.push(OsString::from("-cq"));
        args.push(OsString::from(cq_or_crf));
        args.push(OsString::from("-preset"));
        args.push(OsString::from("p5"));
    } else if encoder.contains("qsv") {
        args.push(OsString::from("-global_quality"));
        args.push(OsString::from(cq_or_crf));
    } else if encoder.contains("amf") {
        args.push(OsString::from("-rc"));
        args.push(OsString::from("cqp"));
        args.push(OsString::from("-qp_i"));
        args.push(OsString::from(cq_or_crf));
        args.push(OsString::from("-qp_p"));
        args.push(OsString::from(cq_or_crf));
    } else if encoder == "libsvtav1" {
        let av1_crf = match q {
            ReencodeQuality::High => "26",
            ReencodeQuality::Medium => "32",
            ReencodeQuality::Small => "38",
        };
        args.push(OsString::from("-crf"));
        args.push(OsString::from(av1_crf));
        args.push(OsString::from("-preset"));
        args.push(OsString::from("8"));
    } else {
        args.push(OsString::from("-crf"));
        args.push(OsString::from(cq_or_crf));
        args.push(OsString::from("-preset"));
        args.push(OsString::from(preset));
    }
}

const TRIM_SILENCE_FILTER: &str = "silenceremove=start_periods=1:start_duration=0.05:start_threshold=-50dB,areverse,silenceremove=start_periods=1:start_duration=0.05:start_threshold=-50dB,areverse";
const OPUS_CODEC: [&str; 4] = ["-c:a", "libopus", "-b:a", "160k"];
const AAC_CODEC: [&str; 4] = ["-c:a", "aac", "-b:a", "256k"];

/// Speed factor, when it really changes the tempo.
fn active_speed(ws: &WorkshopSpec) -> Option<f32> {
    ws.speed_factor.filter(|&v| (v - 1.0).abs() > 0.01)
}

/// Audio filters shared by the audio and video chains, in the order they run. Loudness
/// normalization goes after them, once its measuring pass is done. Silence trimming is audio
/// only: cutting just the soundtrack of a video would pull it out of sync with the picture.
fn audio_filter_chain(
    ws: &WorkshopSpec,
    caps: &FfmpegCapabilities,
    mode: MediaMode,
    duration_s: Option<f64>,
) -> Vec<String> {
    let mut af = Vec::new();
    if mode == MediaMode::Audio && ws.trim_silence && caps.has_silenceremove {
        af.push(TRIM_SILENCE_FILTER.to_string());
    }
    if let Some(semi) = ws.pitch_semitones.filter(|&v| v.abs() > 0.01) {
        if caps.has_rubberband {
            let ratio = 2.0_f32.powf(semi / 12.0);
            af.push(format!("rubberband=pitch={ratio:.6}"));
        }
    }
    let speed = active_speed(ws);
    if let Some(spd) = speed {
        af.extend(build_atempo_chain(spd));
    }
    if let Some(fi) = ws.fade_in_s.filter(|&v| v > 0.0) {
        af.push(format!("afade=t=in:st=0:d={fi:.2}"));
    }
    if let (Some(fo), Some(dur)) = (ws.fade_out_s.filter(|&v| v > 0.0), duration_s) {
        // The fade runs after the speed change, on the shortened (or stretched) timeline.
        let out_dur = dur / f64::from(speed.unwrap_or(1.0));
        let st = (out_dur - fo as f64).max(0.0);
        af.push(format!("afade=t=out:st={st:.2}:d={fo:.2}"));
    }
    af
}

/// Video filters in the order they run: subtitles are burned on the original timeline, the
/// speed change follows (the same factor as the audio `atempo`), and the frame rate comes last.
fn video_filter_chain(ws: &WorkshopSpec, burn_srt: Option<&Path>) -> Vec<String> {
    let mut vf = Vec::new();
    if let Some(h) = ws.scale_height {
        vf.push(format!("scale=-2:{h}:flags=lanczos"));
    }
    if let Some(srt) = burn_srt {
        vf.push(format!("subtitles='{}'", escape_ffmpeg_filter_path(srt)));
    }
    if let Some(spd) = active_speed(ws) {
        vf.push(format!("setpts=PTS/{spd:.4}"));
    }
    if let Some(fps) = ws.target_fps {
        vf.push(format!("fps={fps}"));
    }
    vf
}

/// AAC soundtrack arguments for the video chain, with its filters and channel count.
fn aac_soundtrack_args(af: &[String], channels: Option<u8>, bitrate: &str) -> Vec<OsString> {
    let mut args = Vec::new();
    if !af.is_empty() {
        args.push(OsString::from("-af"));
        args.push(OsString::from(af.join(",")));
    }
    if let Some(ch) = channels {
        args.push(OsString::from("-ac"));
        args.push(OsString::from(ch.clamp(1, 2).to_string()));
    }
    args.extend(["-c:a", "aac", "-b:a", bitrate].map(OsString::from));
    args
}

fn mp3_codec(quality: Mp3Quality) -> Vec<&'static str> {
    match quality {
        Mp3Quality::V0 => vec!["-c:a", "libmp3lame", "-q:a", "0"],
        Mp3Quality::Cbr320 => vec!["-c:a", "libmp3lame", "-b:a", "320k"],
    }
}

fn pcm_le(depth: WavBitDepth) -> &'static str {
    match depth {
        WavBitDepth::S16 => "pcm_s16le",
        WavBitDepth::S24 => "pcm_s24le",
        WavBitDepth::F32 => "pcm_f32le",
    }
}

fn pcm_be(depth: WavBitDepth) -> &'static str {
    match depth {
        WavBitDepth::S16 => "pcm_s16be",
        WavBitDepth::S24 | WavBitDepth::F32 => "pcm_s24be",
    }
}

/// Extension and FFmpeg codec arguments for the audio chain's output. "Original" keeps the
/// downloaded container and takes a codec it accepts: Opus does not fit in `.mp3`, `.m4a` or `.flac`.
fn audio_encoding(spec: &DownloadSpec, current_ext: &str) -> (String, Vec<&'static str>) {
    match spec.audio_format {
        AudioFormat::Wav => ("wav".into(), vec!["-c:a", pcm_le(spec.wav_bit_depth)]),
        AudioFormat::Aiff => ("aiff".into(), vec!["-c:a", pcm_be(spec.wav_bit_depth)]),
        AudioFormat::Flac => ("flac".into(), vec!["-c:a", "flac"]),
        AudioFormat::Alac => ("m4a".into(), vec!["-c:a", "alac"]),
        AudioFormat::Mp3 => ("mp3".into(), mp3_codec(spec.mp3_quality)),
        AudioFormat::M4a => ("m4a".into(), AAC_CODEC.to_vec()),
        AudioFormat::Opus => ("opus".into(), OPUS_CODEC.to_vec()),
        AudioFormat::Original => {
            let codec = match current_ext.to_ascii_lowercase().as_str() {
                "mp3" => mp3_codec(spec.mp3_quality),
                "m4a" | "mp4" | "aac" => AAC_CODEC.to_vec(),
                "flac" => vec!["-c:a", "flac"],
                "wav" => vec!["-c:a", pcm_le(spec.wav_bit_depth)],
                "aiff" | "aif" => vec!["-c:a", pcm_be(spec.wav_bit_depth)],
                "opus" | "ogg" | "oga" | "webm" | "mka" => OPUS_CODEC.to_vec(),
                // A container that may not take Opus becomes `.opus`.
                _ => return ("opus".into(), OPUS_CODEC.to_vec()),
            };
            // Same spelling as on disk: on Windows `a.MP3` and `a.mp3` are one file, and the
            // original is deleted whenever the output path differs from it.
            (current_ext.to_string(), codec)
        }
    }
}

/// Runs all configured FFmpeg Workshop operations in deterministic order on `primary_file`,
/// returning the updated primary file + any sidecar outputs (`.gif`, `.png`, `.srt`).
pub async fn run_workshop_pipeline<F>(
    job_group: &JobGroup,
    bins: &BinManager,
    primary_file: PathBuf,
    spec: &DownloadSpec,
    priority: ProcessPriority,
    mut on_stage_progress: F,
) -> Result<Vec<PathBuf>, String>
where
    F: FnMut(&str, f64),
{
    let ws: &WorkshopSpec = &spec.workshop;
    let caps = bins.status().capabilities;
    let ff = bins.ffmpeg_exe();
    let fp = bins.ffprobe_exe();

    let mut current_path = primary_file;
    let mut extra_outputs: Vec<PathBuf> = Vec::new();
    let mut duration_s = probe_duration_seconds(&fp, &current_path).await;
    // A speed change shortens (or stretches) the output; progress and the target-size bitrate
    // follow the new length.
    let out_duration_s = duration_s.map(|d| d / f64::from(active_speed(ws).unwrap_or(1.0)));

    let audio_filters = audio_filter_chain(ws, &caps, spec.mode, duration_s);
    // Normalizing and changing channels also re-encode the audio.
    let has_audio_filters =
        !audio_filters.is_empty() || ws.loudnorm_lufs.is_some() || ws.channels.is_some();

    // 1. Audio chain (format conversion + trim_silence + speed + pitch + fade + loudnorm + channels + sample_rate/bit_depth)
    let needs_audio_format_conv =
        spec.mode == MediaMode::Audio && spec.audio_format.uses_workshop_conversion();

    if needs_audio_format_conv || (spec.mode == MediaMode::Audio && has_audio_filters) {
        let mut af_parts = audio_filters.clone();

        if let Some(target_lufs) = ws.loudnorm_lufs {
            on_stage_progress(&tr!("Medindo volume (1ª passada)...", "Measuring loudness (1st pass)...", "Midiendo el volumen (1.ª pasada)..."), 0.15);
            let m =
                measure_loudnorm_first_pass(job_group, &ff, &current_path, target_lufs, priority)
                    .await?;
            af_parts.push(format!(
                "loudnorm=I={target_lufs:.1}:TP=-1.0:LRA=11:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true",
                m.input_i, m.input_tp, m.input_lra, m.input_thresh, m.target_offset
            ));
        }

        // High-precision SoXR resampling + triangular_hp dither when reducing bit depth to 16-bit
        let target_sr = if needs_audio_format_conv {
            spec.sample_rate_hz
        } else {
            None
        };
        let reducing_to_16bit = matches!(
            (spec.audio_format, spec.wav_bit_depth),
            (AudioFormat::Wav | AudioFormat::Aiff, WavBitDepth::S16)
        );
        if let Some(sr) = target_sr {
            if caps.has_soxr {
                if reducing_to_16bit {
                    af_parts.push(format!(
                        "aresample=resampler=soxr:precision=28:osr={sr}:dither_method=triangular_hp"
                    ));
                } else {
                    af_parts.push(format!("aresample=resampler=soxr:precision=28:osr={sr}"));
                }
            } else {
                af_parts.push(format!("aresample={sr}"));
            }
        } else if reducing_to_16bit && caps.has_soxr {
            af_parts.push("aresample=dither_method=triangular_hp".to_string());
        }

        let current_ext = current_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        let (target_ext, codec_args) = audio_encoding(spec, &current_ext);

        let stage_label = if ws.loudnorm_lufs.is_some() && !needs_audio_format_conv {
            tr!("Normalizando volume", "Normalizing loudness", "Normalizando el volumen")
        } else {
            let ext = target_ext.to_ascii_uppercase();
            tr!("Convertendo para {ext}", "Converting to {ext}", "Convirtiendo a {ext}")
        };

        let out_final = current_path.with_extension(&target_ext);
        let out_tmp = current_path.with_extension(format!("ws_tmp.{target_ext}"));

        // Check if yt-dlp wrote a square sidecar thumbnail (.png or .jpg) for FLAC/ALAC
        let sidecar_img = if current_path.with_extension("png").is_file() {
            current_path.with_extension("png")
        } else {
            current_path.with_extension("jpg")
        };
        let embed_sidecar_cover = spec.embed_square_cover
            && matches!(spec.audio_format, AudioFormat::Flac | AudioFormat::Alac)
            && sidecar_img.is_file();

        let mut args = vec![OsString::from("-i"), current_path.as_os_str().to_os_string()];
        if embed_sidecar_cover {
            args.push(OsString::from("-i"));
            args.push(sidecar_img.as_os_str().to_os_string());
            args.push(OsString::from("-map"));
            args.push(OsString::from("0:a:0"));
            args.push(OsString::from("-map"));
            args.push(OsString::from("1:v:0"));
            args.push(OsString::from("-c:v"));
            args.push(OsString::from("mjpeg"));
            args.push(OsString::from("-disposition:v:0"));
            args.push(OsString::from("attached_pic"));
        } else {
            args.push(OsString::from("-vn"));
        }

        if !af_parts.is_empty() {
            args.push(OsString::from("-af"));
            args.push(OsString::from(af_parts.join(",")));
        }

        if let Some(ch) = ws.channels {
            args.push(OsString::from("-ac"));
            args.push(OsString::from(ch.clamp(1, 2).to_string()));
        }

        args.extend(codec_args.into_iter().map(OsString::from));

        args.push(out_tmp.as_os_str().to_os_string());
        run_ffmpeg_with_progress(job_group, &ff, &args, out_duration_s, priority, |p| {
            on_stage_progress(&stage_label, p);
        })
        .await?;

        if sidecar_img.is_file() {
            let _ = fs::remove_file(&sidecar_img);
        }
        // The processed file takes its place before the original is deleted, so a failed rename
        // never costs the download.
        fs::rename(&out_tmp, &out_final)
            .map_err(|e| tr!("Erro finalizando arquivo de áudio: {e}", "Error finishing the audio file: {e}", "Error al finalizar el archivo de audio: {e}"))?;
        if current_path != out_final {
            let _ = fs::remove_file(&current_path);
        }
        current_path = out_final;
        duration_s = out_duration_s;
    }

    // 2. Video Workshop operations (Re-encode / Compress to X MB / Scale / FPS / Burn subtitles + Audio filters on video)
    let needs_video_transform = spec.mode == MediaMode::Video
        && (ws.video_reencode.is_some()
            || ws.target_size_mb.is_some()
            || ws.scale_height.is_some()
            || ws.target_fps.is_some()
            || (ws.burn_subtitles && caps.has_libass)
            || has_audio_filters);

    if needs_video_transform {
        let burn_srt = if ws.burn_subtitles && caps.has_libass {
            find_sidecar_srt(&current_path)
        } else {
            None
        };
        let vf_parts = video_filter_chain(ws, burn_srt.as_deref());

        let mut af_parts = audio_filters;
        if let Some(target_lufs) = ws.loudnorm_lufs {
            let m =
                measure_loudnorm_first_pass(job_group, &ff, &current_path, target_lufs, priority)
                    .await?;
            af_parts.push(format!(
                "loudnorm=I={target_lufs:.1}:TP=-1.0:LRA=11:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true",
                m.input_i, m.input_tp, m.input_lra, m.input_thresh, m.target_offset
            ));
        }

        let out_ext = if ws.target_size_mb.is_some() {
            "mp4"
        } else {
            current_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp4")
        };
        let out_final = current_path.with_extension(out_ext);
        let out_tmp = current_path.with_extension(format!("ws_vid.{out_ext}"));

        if let (Some(target_mb), Some(dur)) =
            (ws.target_size_mb, out_duration_s.filter(|&d| d > 0.5))
        {
            // Target size compression with 3% safety margin (Section 8.2)
            let audio_kbps = 128.0_f64;
            let total_bits = (target_mb as f64) * 8.0 * 1024.0 * 1024.0 * 0.97;
            let video_kbps = ((total_bits / dur) / 1000.0 - audio_kbps).max(120.0) as u32;
            let enc = select_video_encoder("h264", false, &caps); // CPU 2-pass gives exact file size!
            let passlog = current_path.with_extension("passlog");

            let mut p1 = vec![
                OsString::from("-i"),
                current_path.as_os_str().to_os_string(),
                OsString::from("-c:v"),
                OsString::from(enc),
                OsString::from("-b:v"),
                OsString::from(format!("{video_kbps}k")),
                OsString::from("-pass"),
                OsString::from("1"),
                OsString::from("-passlogfile"),
                passlog.as_os_str().to_os_string(),
                OsString::from("-an"),
            ];
            if !vf_parts.is_empty() {
                p1.push(OsString::from("-vf"));
                p1.push(OsString::from(vf_parts.join(",")));
            }
            p1.push(OsString::from("-f"));
            p1.push(OsString::from("null"));
            // "-" works on every system (`NUL` is a Windows device name).
            p1.push(OsString::from("-"));

            run_ffmpeg_with_progress(job_group, &ff, &p1, Some(dur), priority, |p| {
                on_stage_progress(&tr!("Comprimindo vídeo (1ª passada)...", "Compressing video (1st pass)...", "Comprimiendo video (1.ª pasada)..."), p * 0.45);
            })
            .await?;

            let mut p2 = vec![
                OsString::from("-i"),
                current_path.as_os_str().to_os_string(),
                OsString::from("-c:v"),
                OsString::from(enc),
                OsString::from("-b:v"),
                OsString::from(format!("{video_kbps}k")),
                OsString::from("-pass"),
                OsString::from("2"),
                OsString::from("-passlogfile"),
                passlog.as_os_str().to_os_string(),
                OsString::from("-movflags"),
                OsString::from("+faststart"),
            ];
            if !vf_parts.is_empty() {
                p2.push(OsString::from("-vf"));
                p2.push(OsString::from(vf_parts.join(",")));
            }
            p2.extend(aac_soundtrack_args(&af_parts, ws.channels, "128k"));
            p2.push(out_tmp.as_os_str().to_os_string());

            let res = run_ffmpeg_with_progress(job_group, &ff, &p2, Some(dur), priority, |p| {
                on_stage_progress(&tr!("Comprimindo vídeo (2ª passada)...", "Compressing video (2nd pass)...", "Comprimiendo video (2.ª pasada)..."), 0.45 + p * 0.55);
            })
            .await;

            // Cleanup 2-pass log files
            for suffix in ["passlog-0.log", "passlog-0.log.mbtree"] {
                if let Some(parent) = passlog.parent() {
                    let stem = passlog.file_stem().unwrap().to_string_lossy();
                    let _ = fs::remove_file(parent.join(suffix.replace("passlog", &stem)));
                }
            }
            res?;
        } else {
            let re_spec = ws.video_reencode.clone().unwrap_or_default();
            let family = if re_spec.codec.is_empty() {
                "h264"
            } else {
                re_spec.codec.as_str()
            };
            let enc = select_video_encoder(family, re_spec.prefer_gpu, &caps);

            let mut args = vec![OsString::from("-i"), current_path.as_os_str().to_os_string()];
            if !vf_parts.is_empty() || ws.video_reencode.is_some() {
                args.push(OsString::from("-c:v"));
                args.push(OsString::from(enc));
                append_quality_flags(&mut args, enc, re_spec.quality);
                if !vf_parts.is_empty() {
                    args.push(OsString::from("-vf"));
                    args.push(OsString::from(vf_parts.join(",")));
                }
            } else {
                args.push(OsString::from("-c:v"));
                args.push(OsString::from("copy"));
            }

            if af_parts.is_empty() && ws.channels.is_none() {
                args.push(OsString::from("-c:a"));
                args.push(OsString::from("copy"));
            } else {
                args.extend(aac_soundtrack_args(&af_parts, ws.channels, "192k"));
            }
            args.push(out_tmp.as_os_str().to_os_string());

            run_ffmpeg_with_progress(job_group, &ff, &args, out_duration_s, priority, |p| {
                on_stage_progress(&tr!("Recodificando vídeo...", "Re-encoding video...", "Recodificando video..."), p);
            })
            .await?;
        }

        fs::rename(&out_tmp, &out_final)
            .map_err(|e| tr!("Erro salvando vídeo processado: {e}", "Error saving the processed video: {e}", "Error al guardar el video procesado: {e}"))?;
        if current_path != out_final {
            let _ = fs::remove_file(&current_path);
        }
        current_path = out_final;
        duration_s = out_duration_s;
    }

    // 3. GIF generation (`palettegen` + `paletteuse` in 2 stages — Section 8.2)
    if let Some(gif) = &ws.make_gif {
        if caps.has_palettegen {
            let fps = gif.fps.clamp(5, 30);
            let width = gif.width.clamp(160, 1080);
            let palette_path = current_path.with_extension("palette.png");
            let gif_path = current_path.with_extension("gif");

            let p1 = vec![
                OsString::from("-i"),
                current_path.as_os_str().to_os_string(),
                OsString::from("-vf"),
                OsString::from(format!("fps={fps},scale={width}:-1:flags=lanczos,palettegen")),
                palette_path.as_os_str().to_os_string(),
            ];
            run_ffmpeg_with_progress(job_group, &ff, &p1, duration_s, priority, |p| {
                on_stage_progress(&tr!("Gerando paleta do GIF...", "Building the GIF palette...", "Generando la paleta del GIF..."), p * 0.4);
            })
            .await?;

            let p2 = vec![
                OsString::from("-i"),
                current_path.as_os_str().to_os_string(),
                OsString::from("-i"),
                palette_path.as_os_str().to_os_string(),
                OsString::from("-lavfi"),
                OsString::from(format!(
                    "fps={fps},scale={width}:-1:flags=lanczos [x]; [x][1:v] paletteuse=dither=bayer:bayer_scale=5"
                )),
                gif_path.as_os_str().to_os_string(),
            ];
            let res = run_ffmpeg_with_progress(job_group, &ff, &p2, duration_s, priority, |p| {
                on_stage_progress(&tr!("Renderizando GIF...", "Rendering GIF...", "Renderizando GIF..."), 0.4 + p * 0.6);
            })
            .await;
            let _ = fs::remove_file(&palette_path);
            res?;
            extra_outputs.push(gif_path);
        }
    }

    // 4. Extract PNG frame (`-ss <t> -frames:v 1`)
    if let Some(ts) = ws.extract_frame_at_s {
        let png_path = current_path.with_extension("frame.png");
        let args = vec![
            OsString::from("-ss"),
            OsString::from(format!("{ts:.3}")),
            OsString::from("-i"),
            current_path.as_os_str().to_os_string(),
            OsString::from("-frames:v"),
            OsString::from("1"),
            png_path.as_os_str().to_os_string(),
        ];
        run_ffmpeg_with_progress(job_group, &ff, &args, Some(1.0), priority, |p| {
            on_stage_progress(&tr!("Extraindo frame PNG...", "Extracting PNG frame...", "Extrayendo fotograma PNG..."), p);
        })
        .await?;
        extra_outputs.push(png_path);
    }

    // 5. Experimental Whisper transcription (`whisper` filter in FFmpeg 9.0.2 — Section 8.3)
    if let Some(wh) = &ws.whisper {
        if caps.has_whisper {
            let model_file = ensure_whisper_model(&wh.model, |st, p| {
                on_stage_progress(st, p * 0.35);
            })
            .await?;
            let srt_path = current_path.with_extension("srt");
            let lang = if wh.language.trim().is_empty() {
                "auto"
            } else {
                wh.language.trim()
            };
            let filter = format!(
                "whisper=model='{}':language='{}':format=srt:destination='{}'",
                escape_ffmpeg_filter_path(&model_file),
                lang,
                escape_ffmpeg_filter_path(&srt_path)
            );
            let args = vec![
                OsString::from("-i"),
                current_path.as_os_str().to_os_string(),
                OsString::from("-vn"),
                OsString::from("-af"),
                OsString::from(filter),
                OsString::from("-f"),
                OsString::from("null"),
                OsString::from("-"),
            ];
            run_ffmpeg_with_progress(job_group, &ff, &args, duration_s, priority, |p| {
                on_stage_progress(&tr!("Transcrevendo áudio (Whisper)...", "Transcribing audio (Whisper)...", "Transcribiendo audio (Whisper)..."), 0.35 + p * 0.65);
            })
            .await?;
            if srt_path.is_file() {
                extra_outputs.push(srt_path);
            }
        }
    }

    let mut all_outputs = vec![current_path];
    all_outputs.extend(extra_outputs);
    Ok(all_outputs)
}

fn find_sidecar_srt(video_path: &Path) -> Option<PathBuf> {
    let direct = video_path.with_extension("srt");
    if direct.is_file() {
        return Some(direct);
    }
    let parent = video_path.parent()?;
    let stem = video_path.file_stem()?.to_string_lossy();
    let entries = fs::read_dir(parent).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("srt")
            && p.file_name()
                .map(|n| n.to_string_lossy().starts_with(stem.as_ref()))
                .unwrap_or(false)
        {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps_with_filters() -> FfmpegCapabilities {
        FfmpegCapabilities {
            has_rubberband: true,
            has_silenceremove: true,
            has_libass: true,
            ..Default::default()
        }
    }

    fn audio_spec(format: AudioFormat) -> DownloadSpec {
        DownloadSpec {
            mode: MediaMode::Audio,
            audio_format: format,
            mp3_quality: Mp3Quality::V0,
            ..Default::default()
        }
    }

    #[test]
    fn original_audio_is_reencoded_with_a_codec_its_container_takes() {
        let spec = audio_spec(AudioFormat::Original);
        let encoding = |ext: &str| audio_encoding(&spec, ext);
        let lame_v0 = vec!["-c:a", "libmp3lame", "-q:a", "0"];

        assert_eq!(encoding("mp3"), ("mp3".to_string(), lame_v0.clone()));
        assert_eq!(encoding("m4a"), ("m4a".to_string(), AAC_CODEC.to_vec()));
        assert_eq!(encoding("flac"), ("flac".to_string(), vec!["-c:a", "flac"]));
        assert_eq!(encoding("opus"), ("opus".to_string(), OPUS_CODEC.to_vec()));
        assert_eq!(encoding("ogg"), ("ogg".to_string(), OPUS_CODEC.to_vec()));
        // The extension keeps its case: on Windows `a.MP3` and `a.mp3` are the same file, and
        // the original is deleted when the output path differs from it.
        assert_eq!(encoding("MP3"), ("MP3".to_string(), lame_v0));
        // A container that may not take Opus becomes `.opus`.
        assert_eq!(encoding("xyz"), ("opus".to_string(), OPUS_CODEC.to_vec()));
        assert_eq!(encoding(""), ("opus".to_string(), OPUS_CODEC.to_vec()));
    }

    #[test]
    fn mp3_keeps_the_chosen_quality_after_the_workshop() {
        let mut spec = audio_spec(AudioFormat::Mp3);
        assert_eq!(audio_encoding(&spec, "mp3").1, vec!["-c:a", "libmp3lame", "-q:a", "0"]);
        spec.mp3_quality = Mp3Quality::Cbr320;
        assert_eq!(audio_encoding(&spec, "mp3").1, vec!["-c:a", "libmp3lame", "-b:a", "320k"]);
    }

    #[test]
    fn video_speed_change_retimes_the_picture_with_the_sound() {
        let ws = WorkshopSpec {
            speed_factor: Some(1.5),
            ..Default::default()
        };
        assert_eq!(video_filter_chain(&ws, None), vec!["setpts=PTS/1.5000"]);
        let af = audio_filter_chain(&ws, &caps_with_filters(), MediaMode::Video, Some(60.0));
        assert_eq!(af, vec!["atempo=1.5000"]);
    }

    #[test]
    fn video_applies_pitch_fades_and_channels_but_never_trims_silence() {
        let ws = WorkshopSpec {
            trim_silence: true,
            pitch_semitones: Some(2.0),
            fade_in_s: Some(1.0),
            fade_out_s: Some(2.0),
            channels: Some(1),
            ..Default::default()
        };
        let caps = caps_with_filters();

        let video = audio_filter_chain(&ws, &caps, MediaMode::Video, Some(60.0));
        assert!(!video.iter().any(|f| f.starts_with("silenceremove")), "{video:?}");
        assert!(video.iter().any(|f| f.starts_with("rubberband=pitch=")), "{video:?}");
        assert!(video.contains(&"afade=t=in:st=0:d=1.00".to_string()), "{video:?}");
        assert!(video.contains(&"afade=t=out:st=58.00:d=2.00".to_string()), "{video:?}");

        let soundtrack: Vec<String> = aac_soundtrack_args(&video, ws.channels, "192k")
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let ac = soundtrack.iter().position(|a| a == "-ac").expect("no channel change");
        assert_eq!(soundtrack[ac + 1], "1");

        // Audio downloads still trim it.
        let audio = audio_filter_chain(&ws, &caps, MediaMode::Audio, Some(60.0));
        assert!(audio[0].starts_with("silenceremove"), "{audio:?}");

        // A video with only silence trimming on does not go through the Workshop at all.
        let only_trim = WorkshopSpec {
            trim_silence: true,
            ..Default::default()
        };
        assert!(!only_trim.has_any_step(MediaMode::Video, AudioFormat::Original));
        assert!(only_trim.has_any_step(MediaMode::Audio, AudioFormat::Original));
    }

    #[test]
    fn fade_out_starts_on_the_sped_up_timeline() {
        let ws = WorkshopSpec {
            speed_factor: Some(1.5),
            fade_out_s: Some(3.0),
            ..Default::default()
        };
        let af = audio_filter_chain(&ws, &FfmpegCapabilities::default(), MediaMode::Audio, Some(60.0));
        // 60 s at 1.5x last 40 s, so the fade starts at 37 s.
        assert_eq!(af.last().map(String::as_str), Some("afade=t=out:st=37.00:d=3.00"));
    }

    #[test]
    fn subtitles_burn_before_the_speed_change_and_fps_is_set_after() {
        let ws = WorkshopSpec {
            scale_height: Some(720),
            target_fps: Some(30),
            speed_factor: Some(2.0),
            ..Default::default()
        };
        let vf = video_filter_chain(&ws, Some(Path::new(r"C:\v\clip.pt.srt")));
        assert_eq!(
            vf,
            vec![
                "scale=-2:720:flags=lanczos",
                r"subtitles='C\:/v/clip.pt.srt'",
                "setpts=PTS/2.0000",
                "fps=30",
            ]
        );
    }
}
