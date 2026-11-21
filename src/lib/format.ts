import {
  siYoutube,
  siYoutubemusic,
  siInstagram,
  siTiktok,
  siX,
  siReddit,
  siTwitch,
  siVimeo,
  siSoundcloud,
  siBandcamp,
  siFacebook,
  siKick,
  siDailymotion,
  siBilibili,
  siSpotify,
  siNetflix,
  siApplemusic,
} from "simple-icons";
import { SourceKind, SpeedUnit } from "./ipc";

const BRAND_PATHS: Record<string, string> = {
  youtube: siYoutube.path,
  youtubemusic: siYoutubemusic.path,
  instagram: siInstagram.path,
  tiktok: siTiktok.path,
  x: siX.path,
  reddit: siReddit.path,
  twitch: siTwitch.path,
  vimeo: siVimeo.path,
  soundcloud: siSoundcloud.path,
  bandcamp: siBandcamp.path,
  facebook: siFacebook.path,
  kick: siKick.path,
  dailymotion: siDailymotion.path,
  bilibili: siBilibili.path,
  spotify: siSpotify.path,
  netflix: siNetflix.path,
  applemusic: siApplemusic.path,
};

export function getBrandSvgPath(slug: string): string | null {
  return BRAND_PATHS[slug] ?? null;
}

export function formatKindPt(kind: SourceKind): string {
  switch (kind) {
    case "video":
      return "vídeo";
    case "short":
      return "Short";
    case "reel":
      return "Reel";
    case "track":
      return "faixa";
    case "album":
      return "álbum";
    case "playlist":
      return "playlist";
    case "playlist_item":
      return "item";
    case "channel":
      return "canal";
    case "live":
      return "live";
    case "clip":
      return "clipe";
    case "story":
      return "Story";
    case "post":
      return "post";
    case "drm":
      return "DRM";
  }
}

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes <= 0) return "—";
  const kb = 1024;
  const mb = kb * 1024;
  const gb = mb * 1024;
  if (bytes >= gb) {
    return `${(bytes / gb).toFixed(2).replace(".", ",")} GB`;
  }
  if (bytes >= mb) {
    const val = bytes / mb;
    return `${(val >= 100 ? val.toFixed(0) : val.toFixed(1)).replace(".", ",")} MB`;
  }
  return `${Math.max(1, Math.round(bytes / kb))} KB`;
}

export function formatSpeedPrimary(bps: number, unit: SpeedUnit = "bytes_per_sec"): string {
  if (!bps || bps < 512) {
    return unit === "bits_per_sec" ? "0,0 Mbps" : "0,0 MB/s";
  }
  if (unit === "bits_per_sec") {
    const mbps = (bps * 8) / 1_000_000;
    if (mbps < 1) return `${Math.round((bps * 8) / 1000)} Kbps`;
    return `${mbps.toFixed(1).replace(".", ",")} Mbps`;
  }
  const mbs = bps / (1024 * 1024);
  if (mbs < 0.95) {
    return `${Math.round(bps / 1024)} KB/s`;
  }
  return `${mbs.toFixed(1).replace(".", ",")} MB/s`;
}

/** Formats bandwidth in BOTH ISP units (Mbps) and OS units (MB/s), e.g. "5,0 MB/s · 42 Mbps" (Section 11.3). */
export function formatDualBandwidth(bps: number, primary: SpeedUnit = "bytes_per_sec"): string {
  const bytesPart = formatSpeedPrimary(bps, "bytes_per_sec");
  const bitsPart = formatSpeedPrimary(bps, "bits_per_sec");
  return primary === "bytes_per_sec" ? `${bytesPart} · ${bitsPart}` : `${bitsPart} · ${bytesPart}`;
}

export function formatDuration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return "—";
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) {
    return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }
  return `${m}:${String(s).padStart(2, "0")}`;
}

export function formatEta(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return "—";
  if (seconds === 0) return "0 s";
  if (seconds < 60) return `${Math.round(seconds)} s`;
  const m = Math.floor(seconds / 60);
  const s = Math.round(seconds % 60);
  if (m < 60) return `${m}m ${String(s).padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  const rm = m % 60;
  return `${h}h ${String(rm).padStart(2, "0")}m`;
}

// Logarithmic bandwidth slider conversion: [0..100] <-> [256 KB/s .. 100 MB/s] (Section 11.3)
const MIN_LIMIT_BPS = 256 * 1024;
const MAX_LIMIT_BPS = 100 * 1024 * 1024;

export function bpsToLogSlider(bps: number): number {
  const clamped = Math.max(MIN_LIMIT_BPS, Math.min(MAX_LIMIT_BPS, bps));
  const ratio = Math.log(clamped / MIN_LIMIT_BPS) / Math.log(MAX_LIMIT_BPS / MIN_LIMIT_BPS);
  return Math.round(ratio * 1000) / 10;
}

export function logSliderToBps(sliderVal: number): number {
  const t = Math.max(0, Math.min(100, sliderVal)) / 100;
  return Math.round(MIN_LIMIT_BPS * Math.pow(MAX_LIMIT_BPS / MIN_LIMIT_BPS, t));
}
