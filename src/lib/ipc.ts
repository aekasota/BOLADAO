import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export type MediaMode = "audio" | "video";
export type SourceKind =
  | "video"
  | "short"
  | "reel"
  | "track"
  | "album"
  | "playlist"
  | "playlist_item"
  | "channel"
  | "live"
  | "clip"
  | "story"
  | "post"
  | "drm";

export type AudioFormat =
  | "original"
  | "mp3"
  | "m4a"
  | "opus"
  | "flac"
  | "alac"
  | "wav"
  | "aiff";

export type Mp3Quality = "v0" | "cbr320";
export type WavBitDepth = "s16" | "s24" | "f32";
export type VideoProfile = "compatible" | "max_quality" | "custom";
export type VideoContainer = "mp4" | "mkv" | "webm" | "mov";
export type SponsorBlockAction = "off" | "remove" | "mark";
export type SubtitleMode = "off" | "embed" | "sidecar_srt";
export type LiveDownloadMode = "from_now" | "from_start" | "wait_for_premiere";
export type ReencodeQuality = "high" | "medium" | "small";
export type CollisionPolicy = "keep_both" | "overwrite" | "skip";
export type BandwidthProfile = "unlimited" | "balanced" | "light" | "custom";
export type GameModeOverride = "auto" | "force_on" | "force_off";
export type ThemeId = "sangue" | "papel" | "acido" | "cobalto" | "breu";
export type LanguageSetting = "auto" | "pt" | "en" | "es";
export type SpeedUnit = "bytes_per_sec" | "bits_per_sec";
export type ProbeScope = "single_video" | "full_playlist";

export interface Classification {
  normalized_url: string;
  dedup_key: string;
  platform_id: string;
  platform_name: string;
  brand_color: string;
  icon_slug: string;
  kind: SourceKind;
  default_mode: MediaMode;
  has_video_and_playlist: boolean;
  requires_login_hint: boolean;
  is_drm: boolean;
  drm_message: string | null;
}

export interface VideoFormatOption {
  height: number;
  fps: number;
  label: string;
  has_hdr: boolean;
  has_h264: boolean;
  vcodecs: string[];
  estimated_total_bytes: number | null;
}

export interface SubtitleTrackInfo {
  lang: string;
  name: string;
  is_auto: boolean;
}

export interface ChapterInfo {
  title: string;
  start_time: number;
  end_time: number;
}

export interface PlaylistEntryPreview {
  index: number;
  id: string;
  url: string;
  title: string;
  uploader: string | null;
  duration_s: number | null;
  thumb_url: string | null;
  is_short: boolean;
  is_live: boolean;
}

export interface ProbeResult {
  url: string;
  classification: Classification;
  extractor_key: string;
  id: string;
  title: string;
  channel_or_artist: string;
  upload_date: string | null;
  duration_s: number | null;
  thumb_url: string | null;
  is_portrait?: boolean;
  is_playlist: boolean;
  playlist_title: string | null;
  playlist_count: number | null;
  entries: PlaylistEntryPreview[];
  video_resolutions: VideoFormatOption[];
  best_audio_bytes: number | null;
  best_audio_acodec: string | null;
  audio_languages: string[];
  manual_subtitles: SubtitleTrackInfo[];
  auto_subtitles: SubtitleTrackInfo[];
  chapters: ChapterInfo[];
  is_live: boolean;
  is_upcoming: boolean;
  release_timestamp: number | null;
}

export interface WorkshopSpec {
  loudnorm_lufs: number | null;
  trim_silence: boolean;
  fade_in_s: number | null;
  fade_out_s: number | null;
  speed_factor: number | null;
  pitch_semitones: number | null;
  channels: number | null;
  video_reencode: {
    codec: string;
    quality: ReencodeQuality;
    prefer_gpu: boolean;
  } | null;
  target_size_mb: number | null;
  scale_height: number | null;
  target_fps: number | null;
  make_gif: { fps: number; width: number } | null;
  extract_frame_at_s: number | null;
  burn_subtitles: boolean;
  whisper: { model: string; language: string } | null;
}

export interface DownloadSpec {
  mode: MediaMode;
  audio_format: AudioFormat;
  mp3_quality: Mp3Quality;
  wav_bit_depth: WavBitDepth;
  sample_rate_hz: number | null;
  embed_square_cover: boolean;
  embed_metadata: boolean;
  parse_artist_title: boolean;
  clean_music_title: boolean;
  video_profile: VideoProfile;
  max_height: number | null;
  custom_container: VideoContainer;
  custom_vcodec: string | null;
  audio_language: string | null;
  subtitle_mode: SubtitleMode;
  subtitle_langs: string[];
  include_auto_subs: boolean;
  embed_chapters: boolean;
  sponsorblock_action: SponsorBlockAction;
  sponsorblock_categories: string[];
  section_start_s: number | null;
  section_end_s: number | null;
  force_keyframes_at_cuts: boolean;
  split_chapters: boolean;
  concat_playlist: boolean;
  live_mode: LiveDownloadMode;
  output_root: string;
  output_template: string;
  playlist_numbering: boolean;
  use_download_archive: boolean;
  workshop: WorkshopSpec;
  extra_ytdlp_args: string[];
}

export interface Preset {
  id: string;
  name: string;
  description: string;
  builtin: boolean;
  spec: DownloadSpec;
}

export type ErrorAction =
  | { type: "none" }
  | { type: "retry" }
  | { type: "update_ytdlp" }
  | { type: "login"; payload: { platform: string } }
  | { type: "change_format" }
  | { type: "change_destination" };

export interface JobError {
  code: string;
  message_pt: string;
  hint_pt: string | null;
  action: ErrorAction;
  transient: boolean;
  suggest_impersonate: boolean;
}

export type JobState =
  | "queued"
  | "probing"
  | "waiting_premiere"
  | "downloading"
  | "postprocessing"
  | "workshop"
  | "done"
  | "failed"
  | "paused"
  | "canceled";

export interface Progress {
  stage: string;
  pct: number;
  downloaded_bytes: number;
  total_bytes: number | null;
  approximate: boolean;
  speed_bps: number;
  eta_s: number | null;
}

export interface Job {
  id: string;
  url: string;
  dedup_key: string;
  source: {
    platform_id: string;
    platform_name: string;
    extractor_key: string;
    kind: SourceKind;
    brand_color: string;
    icon_slug: string;
  };
  group_id: string | null;
  group_title: string | null;
  playlist_index: number | null;
  title: string;
  channel_or_artist: string;
  thumb_url: string | null;
  duration_s: number | null;
  estimated_bytes: number | null;
  spec: DownloadSpec;
  state: JobState;
  progress: Progress;
  outputs: string[];
  error: JobError | null;
  attempts: number;
  priority: number;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  log_tail: string[];
  equivalent_cmd: string;
}

export interface MediaProbeDetails {
  container_format: string | null;
  file_size_bytes: number;
  duration_s: number | null;
  video_codec: string | null;
  width: number | null;
  height: number | null;
  fps: number | null;
  audio_codec: string | null;
  sample_rate_hz: number | null;
  bits_per_sample: number | null;
  channels: number | null;
  overall_bitrate_kbps: number | null;
  cover_dimensions: [number, number] | null;
  integrated_lufs: number | null;
  lossy_cutoff_note: string | null;
}

export interface LibraryEntry {
  id: string;
  url: string;
  dedup_key: string;
  platform_id: string;
  platform_name: string;
  title: string;
  channel_or_artist: string;
  thumb_url: string | null;
  is_playlist_origin: boolean;
  playlist_url: string | null;
  mode: MediaMode;
  format_badge: string;
  primary_path: string;
  extra_paths: string[];
  file_exists: boolean;
  waveform_url: string | null;
  spectrogram_url: string | null;
  details: MediaProbeDetails;
  equivalent_cmd: string;
  spec: DownloadSpec;
  created_at: string;
}

export interface ConnectedAccount {
  platform_id: string;
  auth_kind: "embedded_cookies" | "imported_cookies_file" | "firefox_browser";
  connected_at: string;
  earliest_expiry_ts: number | null;
  cookie_count: number;
}

export interface PlatformPathRule {
  platform: string;
  enabled: boolean;
  custom_root: string | null;
  template: string;
}

export interface AppSettings {
  music_root: string;
  video_root: string;
  audio_template: string;
  video_template: string;
  playlist_audio_template: string;
  playlist_video_template: string;
  platform_rules: PlatformPathRule[];
  recent_folders: string[];
  favorite_folders: string[];
  collision_policy: CollisionPolicy;
  migrated_from_yt2: string | null;
  migration_banner_dismissed: boolean;
  global_concurrency: number;
  host_concurrency: Record<string, number>;
  bandwidth_profile: BandwidthProfile;
  custom_limit_bps: number;
  light_limit_bps: number;
  peak_7d_bps: number;
  speed_unit: SpeedUnit;
  tray_notice_shown: boolean;
  game_mode_auto: boolean;
  game_mode_override: GameModeOverride;
  game_exe_list: string[];
  game_bandwidth_profile: BandwidthProfile;
  game_custom_limit_bps: number;
  game_concurrency: number;
  game_pause_workshop: boolean;
  accounts: Record<string, ConnectedAccount>;
  custom_presets: Preset[];
  last_spec_by_platform: Record<string, DownloadSpec>;
  theme: ThemeId;
  language: LanguageSetting;
  shader_intensity: number;
  shader_outside_intensity: number;
  shader_stripe_density: number;
  reduce_motion: boolean;
  ytdlp_channel: string;
  last_ytdlp_check: string | null;
  global_extra_ytdlp_args: string[];
  log_level: string;
}

export interface FfmpegCapabilities {
  ffmpeg_version: string;
  has_whisper: boolean;
  has_rubberband: boolean;
  has_soxr: boolean;
  has_libass: boolean;
  has_loudnorm: boolean;
  has_silenceremove: boolean;
  has_showwavespic: boolean;
  has_showspectrumpic: boolean;
  has_palettegen: boolean;
  hw_encoders: string[];
  sw_encoders: string[];
}

export interface BinsStatusPayload {
  ready: boolean;
  extracting: boolean;
  stage: string;
  component: string | null;
  progress_pct: number;
  ytdlp_version: string;
  ytdlp_previous_version: string | null;
  ytdlp_channel: string;
  deno_version: string;
  ffmpeg_version: string;
  update_available: string | null;
  capabilities: FfmpegCapabilities;
  error: string | null;
}

export interface LimiterStatsPayload {
  speed_bps: number;
  limit_bps: number;
  profile: BandwidthProfile;
  peak_7d_bps: number;
  sparkline_60s: number[];
  proxy_port: number | null;
  fallback_mode: boolean;
  game_mode_override_active: boolean;
}

export interface AppUpdateInfo {
  version: string;
  notes: string | null;
}

export interface GameModeStatus {
  active: boolean;
  override_mode: GameModeOverride;
  reason: string | null;
}

export type EnqueueOutcome =
  | { status: "enqueued"; jobs: Job[] }
  | { status: "already_queued"; existing_id: string }
  | { status: "already_in_library"; entry: LibraryEntry };

export interface EnqueueRequest {
  url: string;
  title?: string | null;
  channel_or_artist?: string | null;
  thumb_url?: string | null;
  duration_s?: number | null;
  estimated_bytes?: number | null;
  extractor_key?: string | null;
  spec: DownloadSpec;
  high_priority: boolean;
  force_duplicate: boolean;
  group_id?: string | null;
  group_title?: string | null;
  playlist_index?: number | null;
}

/** Maps a BCP 47 tag to a supported language; anything else falls back to English. */
export function languageFromTag(tag: string | undefined): "pt" | "en" | "es" {
  const t = (tag ?? "").toLowerCase();
  if (t.startsWith("pt")) return "pt";
  if (t.startsWith("es")) return "es";
  return "en";
}

export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function defaultWorkshopSpec(): WorkshopSpec {
  return {
    loudnorm_lufs: null,
    trim_silence: false,
    fade_in_s: null,
    fade_out_s: null,
    speed_factor: null,
    pitch_semitones: null,
    channels: null,
    video_reencode: null,
    target_size_mb: null,
    scale_height: null,
    target_fps: null,
    make_gif: null,
    extract_frame_at_s: null,
    burn_subtitles: false,
    whisper: null,
  };
}

export function createDefaultDownloadSpec(
  mode: MediaMode = "video",
  musicRoot = "C:\\Users\\Usuario\\Music\\Boladão",
  videoRoot = "C:\\Users\\Usuario\\Videos\\Boladão"
): DownloadSpec {
  return {
    mode,
    audio_format: "original",
    mp3_quality: "v0",
    wav_bit_depth: "s24",
    sample_rate_hz: 48000,
    embed_square_cover: true,
    embed_metadata: true,
    parse_artist_title: true,
    clean_music_title: true,
    video_profile: "compatible",
    max_height: 1080,
    custom_container: "mp4",
    custom_vcodec: null,
    audio_language: null,
    subtitle_mode: "off",
    subtitle_langs: ["pt.*", "en.*"],
    include_auto_subs: false,
    embed_chapters: true,
    sponsorblock_action: "off",
    sponsorblock_categories: ["sponsor"],
    section_start_s: null,
    section_end_s: null,
    force_keyframes_at_cuts: false,
    split_chapters: false,
    concat_playlist: false,
    live_mode: "from_now",
    output_root: mode === "audio" ? musicRoot : videoRoot,
    output_template:
      mode === "audio"
        ? "{artista}\\{album}\\{faixa}"
        : "{plataforma}\\{canal}\\{titulo} [{id}]",
    playlist_numbering: false,
    use_download_archive: false,
    workshop: defaultWorkshopSpec(),
    extra_ytdlp_args: [],
  };
}

// Typed IPC wrappers with clean standalone fallback when previewing outside Tauri
export const api = {
  getBinsStatus: (): Promise<BinsStatusPayload> =>
    isTauriRuntime()
      ? invoke("get_bins_status")
      : Promise.resolve({
          ready: true,
          extracting: false,
          stage: "Pronto",
          component: null,
          progress_pct: 1,
          ytdlp_version: "2026.10.26",
          ytdlp_previous_version: null,
          ytdlp_channel: "stable",
          deno_version: "2.9.8",
          ffmpeg_version: "9.0.2",
          update_available: null,
          capabilities: {
            ffmpeg_version: "9.0.2",
            has_whisper: true,
            has_rubberband: true,
            has_soxr: true,
            has_libass: true,
            has_loudnorm: true,
            has_silenceremove: true,
            has_showwavespic: true,
            has_showspectrumpic: true,
            has_palettegen: true,
            hw_encoders: ["h264_nvenc", "hevc_nvenc"],
            sw_encoders: ["libx264", "libx265", "libsvtav1", "libmp3lame", "libopus", "flac", "alac"],
          },
          error: null,
        }),

  runYtdlpDiagnostic: (): Promise<string> =>
    isTauriRuntime()
      ? invoke("run_ytdlp_diagnostic")
      : Promise.resolve(
          "[debug] yt-dlp version stable@2026.10.26\n[debug] exe versions: ffmpeg 9.0.2-full_build-www.gyan.dev, ffprobe 9.0.2\n[debug] JS runtimes: deno-2.9.8"
        ),

  listSupportedSites: (): Promise<string[]> =>
    isTauriRuntime()
      ? invoke("list_supported_sites")
      : Promise.resolve(["youtube", "youtube:playlist", "instagram", "tiktok", "soundcloud", "bandcamp", "twitter", "twitch:vod", "vimeo", "reddit"]),

  checkYtdlpUpdate: (): Promise<string | null> =>
    isTauriRuntime() ? invoke("check_ytdlp_update") : Promise.resolve(null),

  updateYtdlp: (channel?: string): Promise<string> =>
    isTauriRuntime() ? invoke("update_ytdlp", { channel }) : Promise.resolve("2026.10.26"),

  rollbackYtdlp: (): Promise<string> =>
    isTauriRuntime() ? invoke("rollback_ytdlp") : Promise.resolve("2026.10.26"),

  getSystemLanguage: (): Promise<{ language: "pt" | "en" | "es"; locale: string }> =>
    isTauriRuntime()
      ? invoke("get_system_language")
      : Promise.resolve({ language: languageFromTag(navigator.language), locale: navigator.language }),

  getPendingUpdate: (): Promise<AppUpdateInfo | null> =>
    isTauriRuntime() ? invoke("get_pending_update") : Promise.resolve(null),

  applyUpdateNow: (): Promise<void> => invoke("apply_update_now"),

  openAuthorProfile: (): Promise<void> => {
    if (isTauriRuntime()) return invoke("open_author_profile");
    window.open("https://github.com/aekasota", "_blank", "noopener");
    return Promise.resolve();
  },

  classifyUrl: (url: string): Promise<Classification> =>
    isTauriRuntime() ? invoke("classify_url_cmd", { url }) : localClassifyFallback(url),

  extractUrls: (text: string): Promise<string[]> =>
    isTauriRuntime()
      ? invoke("extract_urls_cmd", { text })
      : Promise.resolve(Array.from(text.matchAll(/https?:\/\/[^\s<>"'`)\]]+/g)).map((m) => m[0].replace(/[.,;!?]+$/, ""))),

  probe: (url: string, scope: ProbeScope = "single_video"): Promise<ProbeResult> =>
    isTauriRuntime() ? invoke("probe", { url, scope }) : Promise.reject({ message_pt: "Sondagem disponível no app desktop." }),

  previewPath: (
    rootDir: string,
    template: string,
    sample: {
      media_type: string;
      platform: string;
      creator: string;
      playlist: string | null;
      index: number | null;
      title: string;
      track: string | null;
      id: string;
      year: string;
      month: string;
      ext: string;
    }
  ): Promise<string> =>
    isTauriRuntime()
      ? invoke("preview_path_cmd", { rootDir, template, sample })
      : Promise.resolve(
          `${rootDir}\\${template
            .replace("{plataforma}", sample.platform)
            .replace("{canal}", sample.creator)
            .replace("{artista}", sample.creator)
            .replace("{album}", sample.playlist || "Singles")
            .replace("{playlist}", sample.playlist || "Playlist")
            .replace("{titulo}", sample.title)
            .replace("{faixa}", sample.track || sample.title)
            .replace("{id}", sample.id)
            .replace("{nn}", String(sample.index ?? 1).padStart(2, "0"))}.${sample.ext}`
        ),

  getFreeDiskSpace: (path: string): Promise<number | null> =>
    isTauriRuntime() ? invoke("get_free_disk_space_cmd", { path }) : Promise.resolve(184 * 1024 * 1024 * 1024),

  enqueue: (req: EnqueueRequest): Promise<EnqueueOutcome> =>
    invoke("enqueue", { req }),

  enqueueBatch: (urls: string[], spec: DownloadSpec, highPriority: boolean): Promise<Job[]> =>
    invoke("enqueue_batch", { urls, spec, highPriority }),

  enqueuePlaylist: (
    playlistTitle: string,
    platformUrl: string,
    items: PlaylistEntryPreview[],
    spec: DownloadSpec,
    highPriority: boolean
  ): Promise<Job[]> =>
    invoke("enqueue_playlist", { playlistTitle, platformUrl, items, spec, highPriority }),

  getQueue: (): Promise<Job[]> =>
    isTauriRuntime() ? invoke("get_queue") : Promise.resolve([]),

  pause: (id: string): Promise<void> => invoke("pause", { id }),
  resume: (id: string): Promise<void> => invoke("resume", { id }),
  cancel: (id: string): Promise<void> => invoke("cancel", { id }),
  retry: (id: string): Promise<void> => invoke("retry", { id }),
  removeJobs: (ids: string[]): Promise<void> => invoke("remove_jobs_cmd", { ids }),
  pauseGroup: (groupId: string): Promise<void> => invoke("pause_group_cmd", { groupId }),
  resumeGroup: (groupId: string): Promise<void> => invoke("resume_group_cmd", { groupId }),
  pauseAll: (): Promise<void> => (isTauriRuntime() ? invoke("pause_all_cmd") : Promise.resolve()),
  resumeAll: (): Promise<void> => (isTauriRuntime() ? invoke("resume_all_cmd") : Promise.resolve()),
  clearCompleted: (): Promise<void> => (isTauriRuntime() ? invoke("clear_completed_cmd") : Promise.resolve()),
  reorder: (orderedIds: string[]): Promise<void> => invoke("reorder", { orderedIds }),
  setConcurrency: (concurrency: number): Promise<void> =>
    isTauriRuntime() ? invoke("set_concurrency", { concurrency }) : Promise.resolve(),

  setLimit: (profile: BandwidthProfile, customBps?: number): Promise<LimiterStatsPayload> =>
    invoke("set_limit", { profile, customBps: customBps ?? null }),

  getGameModeStatus: (): Promise<GameModeStatus> =>
    isTauriRuntime()
      ? invoke("get_gamemode_status")
      : Promise.resolve({ active: false, override_mode: "auto", reason: null }),

  setGameModeOverride: (overrideMode: GameModeOverride): Promise<GameModeStatus> =>
    isTauriRuntime()
      ? invoke("set_gamemode_override", { overrideMode })
      : Promise.resolve({ active: overrideMode === "force_on", override_mode: overrideMode, reason: overrideMode === "force_on" ? "Ativado manualmente" : null }),

  listRunningProcesses: (): Promise<string[]> =>
    isTauriRuntime() ? invoke("list_running_processes_cmd") : Promise.resolve(["explorer.exe", "discord.exe", "steam.exe"]),

  equivalentCommand: (jobId: string): Promise<string> =>
    invoke("equivalent_command", { jobId }),

  openFile: (path: string): Promise<void> => invoke("open_file", { path }),
  revealInExplorer: (path: string): Promise<void> => invoke("reveal_in_explorer_cmd", { path }),
  openDataFolder: (which: "roaming" | "local"): Promise<void> => invoke("open_data_folder_cmd", { which }),
  startFileDrag: (path: string): Promise<void> =>
    isTauriRuntime() ? invoke("start_file_drag", { path }) : Promise.resolve(),

  getSettings: (): Promise<AppSettings> =>
    isTauriRuntime() ? invoke("get_settings") : Promise.resolve(fallbackSettings()),

  saveSettings: (settings: AppSettings): Promise<void> =>
    isTauriRuntime() ? invoke("save_settings", { settings }) : Promise.resolve(),

  getPresets: (): Promise<Preset[]> =>
    isTauriRuntime() ? invoke("get_presets") : Promise.resolve([]),

  saveCustomPreset: (name: string, description: string, spec: DownloadSpec): Promise<Preset> =>
    invoke("save_custom_preset", { name, description, spec }),

  deleteCustomPreset: (presetId: string): Promise<void> =>
    invoke("delete_custom_preset", { presetId }),

  loginPlatform: (platformId: string): Promise<string> =>
    invoke("login_platform", { platformId }),

  finishLoginPlatform: (platformId: string): Promise<ConnectedAccount> =>
    invoke("finish_login_platform", { platformId }),

  connectFirefoxCookies: (platformId: string): Promise<ConnectedAccount> =>
    invoke("connect_firefox_cookies", { platformId }),

  importCookiesFile: (platformId: string, filePath: string): Promise<ConnectedAccount> =>
    invoke("import_cookies_file_cmd", { platformId, filePath }),

  logoutPlatform: (platformId: string): Promise<void> =>
    invoke("logout_platform", { platformId }),

  queryLibrary: (query: {
    search?: string | null;
    mode?: MediaMode | null;
    platform_id?: string | null;
    days_back?: number | null;
  }): Promise<LibraryEntry[]> =>
    isTauriRuntime() ? invoke("query_library", { query }) : Promise.resolve([]),

  deleteLibraryEntry: (id: string, deleteDiskFile: boolean): Promise<void> =>
    invoke("delete_library_entry", { id, deleteDiskFile }),

  generateSpectrogram: (id: string): Promise<LibraryEntry> =>
    invoke("generate_spectrogram", { id }),

  runWorkshop: (entryId: string, spec: DownloadSpec): Promise<LibraryEntry> =>
    invoke("run_workshop", { entryId, spec }),

  getLaunchMode: (): Promise<LaunchMode> =>
    isTauriRuntime() ? invoke("get_launch_mode") : Promise.resolve({ mode: "app" }),

  getInstallerDefaults: (): Promise<InstallerDefaults> =>
    isTauriRuntime()
      ? invoke("get_installer_defaults")
      : Promise.resolve({
          default_install_dir: "C:\\Users\\Usuario\\AppData\\Local\\Programs\\Boladão",
          already_installed: false,
          app_version: "1.0.0",
          music_dir: "C:\\Users\\Usuario\\Music\\Boladão",
          video_dir: "C:\\Users\\Usuario\\Videos\\Boladão",
        }),

  installBoladao: (
    installDir: string,
    createDesktopShortcut: boolean,
    createStartMenuShortcut: boolean
  ): Promise<string> =>
    invoke("install_boladao", {
      installDir,
      createDesktopShortcut,
      createStartMenuShortcut,
    }),

  launchInstalledBoladao: (installedExe: string): Promise<void> =>
    invoke("launch_installed_boladao", { installedExe }),

  uninstallBoladao: (targetDir: string, removeUserData: boolean): Promise<void> =>
    invoke("uninstall_boladao", { targetDir, removeUserData }),

  closeUninstaller: (): Promise<void> => invoke("close_uninstaller"),
};

export type LaunchMode =
  | { mode: "app" }
  | { mode: "installer" }
  | { mode: "uninstaller"; target_dir: string };

export interface InstallerDefaults {
  default_install_dir: string;
  already_installed: boolean;
  app_version: string;
  music_dir: string;
  video_dir: string;
}

export interface InstallerProgressPayload {
  pct: number;
  stage: string;
  done: boolean;
  installed_exe: string | null;
  error: string | null;
}

export function readInitialLaunchMode(): LaunchMode {
  const injected = (window as unknown as { __BOLADAO_LAUNCH__?: LaunchMode })
    .__BOLADAO_LAUNCH__;
  if (injected && typeof injected.mode === "string") {
    return injected;
  }
  return { mode: "app" };
}

export async function safeListen<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  if (!isTauriRuntime()) {
    return () => {};
  }
  return listen<T>(event, (ev) => handler(ev.payload));
}

function localClassifyFallback(raw: string): Promise<Classification> {
  try {
    const u = new URL(raw.trim());
    const h = u.hostname.replace(/^(www\.|m\.)/, "").toLowerCase();
    if (h.includes("spotify.com") || h.includes("netflix.com")) {
      return Promise.resolve({
        normalized_url: u.toString(),
        dedup_key: u.toString(),
        platform_id: "spotify",
        platform_name: h.includes("spotify") ? "Spotify" : "Netflix",
        brand_color: "#1DB954",
        icon_slug: "spotify",
        kind: "drm",
        default_mode: "audio",
        has_video_and_playlist: false,
        requires_login_hint: false,
        is_drm: true,
        drm_message: "Esse serviço usa DRM. O yt-dlp não baixa daqui.",
      });
    }
    const isYtMusic = u.hostname === "music.youtube.com";
    const isYt = h === "youtube.com" || h === "youtu.be";
    return Promise.resolve({
      normalized_url: u.toString(),
      dedup_key: u.toString(),
      platform_id: isYtMusic ? "youtube_music" : isYt ? "youtube" : "generic",
      platform_name: isYtMusic ? "YouTube Music" : isYt ? "YouTube" : h,
      brand_color: isYt || isYtMusic ? "#FF0033" : "#A8A39C",
      icon_slug: isYtMusic ? "youtubemusic" : isYt ? "youtube" : "globe",
      kind: isYtMusic ? "track" : "video",
      default_mode: isYtMusic ? "audio" : "video",
      has_video_and_playlist: u.searchParams.has("v") && u.searchParams.has("list"),
      requires_login_hint: false,
      is_drm: false,
      drm_message: null,
    });
  } catch {
    return Promise.reject("Link inválido.");
  }
}

export function fallbackSettings(): AppSettings {
  return {
    music_root: "C:\\Users\\Usuario\\Music\\Boladão",
    video_root: "C:\\Users\\Usuario\\Videos\\Boladão",
    audio_template: "{artista}\\{album}\\{faixa}",
    video_template: "{plataforma}\\{canal}\\{titulo} [{id}]",
    playlist_audio_template: "{artista}\\{playlist}\\{nn} - {faixa}",
    playlist_video_template: "{plataforma}\\{canal}\\{playlist}\\{nn} - {titulo}",
    platform_rules: [],
    recent_folders: [],
    favorite_folders: [],
    collision_policy: "keep_both",
    migrated_from_yt2: null,
    migration_banner_dismissed: false,
    global_concurrency: 3,
    host_concurrency: { youtube: 2, instagram: 1, tiktok: 1 },
    bandwidth_profile: "unlimited",
    custom_limit_bps: 5 * 1024 * 1024,
    light_limit_bps: 2 * 1024 * 1024,
    peak_7d_bps: 0,
    speed_unit: "bytes_per_sec",
    tray_notice_shown: false,
    game_mode_auto: true,
    game_mode_override: "auto",
    game_exe_list: ["cs2.exe", "valorant-win64-shipping.exe"],
    game_bandwidth_profile: "light",
    game_custom_limit_bps: 2 * 1024 * 1024,
    game_concurrency: 1,
    game_pause_workshop: false,
    accounts: {},
    custom_presets: [],
    last_spec_by_platform: {},
    theme: "sangue",
    language: "auto",
    shader_intensity: 1.0,
    shader_outside_intensity: 1.0,
    shader_stripe_density: 1.0,
    reduce_motion: false,
    ytdlp_channel: "stable",
    last_ytdlp_check: null,
    global_extra_ytdlp_args: [],
    log_level: "info",
  };
}
