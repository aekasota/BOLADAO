use crate::bins::{BinManager, FfmpegCapabilities};
use crate::classify::MediaMode;
use crate::paths::app_local_dir;
use crate::spec::{AudioFormat, DownloadSpec, ReencodeQuality, WavBitDepth, WorkshopSpec};
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
            "hevc" | "h265" => &["hevc_nvenc", "hevc_qsv", "hevc_amf"],
            "av1" => &["av1_nvenc", "av1_qsv", "av1_amf"],
            _ => &["h264_nvenc", "h264_qsv", "h264_amf"],
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
    if encoder.contains("nvenc") {
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
    let duration_s = probe_duration_seconds(&fp, &current_path).await;

    // 1. Audio chain (format conversion + trim_silence + speed + pitch + fade + loudnorm + channels + sample_rate/bit_depth)
    let needs_audio_format_conv =
        spec.mode == MediaMode::Audio && spec.audio_format.uses_workshop_conversion();
    let has_audio_filters = ws.loudnorm_lufs.is_some()
        || ws.trim_silence
        || ws.fade_in_s.filter(|&v| v > 0.0).is_some()
        || ws.fade_out_s.filter(|&v| v > 0.0).is_some()
        || ws.speed_factor.filter(|&v| (v - 1.0).abs() > 0.01).is_some()
        || (ws.pitch_semitones.filter(|&v| v.abs() > 0.01).is_some() && caps.has_rubberband)
        || ws.channels.is_some()
        || (spec.mode == MediaMode::Audio
            && spec.sample_rate_hz.is_some()
            && needs_audio_format_conv);

    if needs_audio_format_conv || (spec.mode == MediaMode::Audio && has_audio_filters) {
        let mut af_parts: Vec<String> = Vec::new();

        if ws.trim_silence && caps.has_silenceremove {
            af_parts.push(
                "silenceremove=start_periods=1:start_duration=0.05:start_threshold=-50dB,areverse,silenceremove=start_periods=1:start_duration=0.05:start_threshold=-50dB,areverse"
                    .to_string(),
            );
        }

        if let Some(semi) = ws.pitch_semitones.filter(|&v| v.abs() > 0.01) {
            if caps.has_rubberband {
                let ratio = 2.0_f32.powf(semi / 12.0);
                af_parts.push(format!("rubberband=pitch={ratio:.6}"));
            }
        }

        if let Some(spd) = ws.speed_factor.filter(|&v| (v - 1.0).abs() > 0.01) {
            af_parts.extend(build_atempo_chain(spd));
        }

        if let Some(fi) = ws.fade_in_s.filter(|&v| v > 0.0) {
            af_parts.push(format!("afade=t=in:st=0:d={fi:.2}"));
        }
        if let (Some(fo), Some(dur)) = (ws.fade_out_s.filter(|&v| v > 0.0), duration_s) {
            let st = (dur - fo as f64).max(0.0);
            af_parts.push(format!("afade=t=out:st={st:.2}:d={fo:.2}"));
        }

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

        let target_ext = match spec.audio_format {
            AudioFormat::Wav => "wav",
            AudioFormat::Aiff => "aiff",
            AudioFormat::Flac => "flac",
            AudioFormat::Alac => "m4a",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::M4a => "m4a",
            AudioFormat::Opus => "opus",
            AudioFormat::Original => current_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("opus"),
        };

        let stage_label = if ws.loudnorm_lufs.is_some() && !needs_audio_format_conv {
            tr!("Normalizando volume", "Normalizing loudness", "Normalizando el volumen")
        } else {
            let ext = target_ext.to_ascii_uppercase();
            tr!("Convertendo para {ext}", "Converting to {ext}", "Convirtiendo a {ext}")
        };

        let out_final = current_path.with_extension(target_ext);
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

        match spec.audio_format {
            AudioFormat::Wav => {
                let pcm = match spec.wav_bit_depth {
                    WavBitDepth::S16 => "pcm_s16le",
                    WavBitDepth::S24 => "pcm_s24le",
                    WavBitDepth::F32 => "pcm_f32le",
                };
                args.push(OsString::from("-c:a"));
                args.push(OsString::from(pcm));
            }
            AudioFormat::Aiff => {
                let pcm = match spec.wav_bit_depth {
                    WavBitDepth::S16 => "pcm_s16be",
                    WavBitDepth::S24 | WavBitDepth::F32 => "pcm_s24be",
                };
                args.push(OsString::from("-c:a"));
                args.push(OsString::from(pcm));
            }
            AudioFormat::Flac => {
                args.push(OsString::from("-c:a"));
                args.push(OsString::from("flac"));
            }
            AudioFormat::Alac => {
                args.push(OsString::from("-c:a"));
                args.push(OsString::from("alac"));
            }
            AudioFormat::Mp3 => {
                args.push(OsString::from("-c:a"));
                args.push(OsString::from("libmp3lame"));
                args.push(OsString::from("-q:a"));
                args.push(OsString::from("0"));
            }
            AudioFormat::M4a => {
                args.push(OsString::from("-c:a"));
                args.push(OsString::from("aac"));
                args.push(OsString::from("-b:a"));
                args.push(OsString::from("256k"));
            }
            AudioFormat::Opus | AudioFormat::Original => {
                args.push(OsString::from("-c:a"));
                args.push(OsString::from("libopus"));
                args.push(OsString::from("-b:a"));
                args.push(OsString::from("160k"));
            }
        }

        args.push(out_tmp.as_os_str().to_os_string());
        run_ffmpeg_with_progress(job_group, &ff, &args, duration_s, priority, |p| {
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
        let mut vf_parts: Vec<String> = Vec::new();
        if let Some(h) = ws.scale_height {
            vf_parts.push(format!("scale=-2:{h}:flags=lanczos"));
        }
        if let Some(fps) = ws.target_fps {
            vf_parts.push(format!("fps={fps}"));
        }
        if ws.burn_subtitles && caps.has_libass {
            if let Some(srt) = find_sidecar_srt(&current_path) {
                let escaped = escape_ffmpeg_filter_path(&srt);
                vf_parts.push(format!("subtitles='{escaped}'"));
            }
        }

        let mut af_parts: Vec<String> = Vec::new();
        if let Some(target_lufs) = ws.loudnorm_lufs {
            let m =
                measure_loudnorm_first_pass(job_group, &ff, &current_path, target_lufs, priority)
                    .await?;
            af_parts.push(format!(
                "loudnorm=I={target_lufs:.1}:TP=-1.0:LRA=11:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true",
                m.input_i, m.input_tp, m.input_lra, m.input_thresh, m.target_offset
            ));
        }
        if let Some(spd) = ws.speed_factor.filter(|&v| (v - 1.0).abs() > 0.01) {
            af_parts.extend(build_atempo_chain(spd));
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

        if let (Some(target_mb), Some(dur)) = (ws.target_size_mb, duration_s.filter(|&d| d > 0.5))
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
            p1.push(OsString::from("NUL"));

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
                OsString::from("-c:a"),
                OsString::from("aac"),
                OsString::from("-b:a"),
                OsString::from("128k"),
                OsString::from("-movflags"),
                OsString::from("+faststart"),
            ];
            if !vf_parts.is_empty() {
                p2.push(OsString::from("-vf"));
                p2.push(OsString::from(vf_parts.join(",")));
            }
            if !af_parts.is_empty() {
                p2.push(OsString::from("-af"));
                p2.push(OsString::from(af_parts.join(",")));
            }
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

            if !af_parts.is_empty() {
                args.push(OsString::from("-af"));
                args.push(OsString::from(af_parts.join(",")));
                args.push(OsString::from("-c:a"));
                args.push(OsString::from("aac"));
                args.push(OsString::from("-b:a"));
                args.push(OsString::from("192k"));
            } else {
                args.push(OsString::from("-c:a"));
                args.push(OsString::from("copy"));
            }
            args.push(out_tmp.as_os_str().to_os_string());

            run_ffmpeg_with_progress(job_group, &ff, &args, duration_s, priority, |p| {
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
