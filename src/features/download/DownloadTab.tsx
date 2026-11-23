import React, { useEffect, useMemo, useRef, useState } from "react";
import { MOD_KEY } from "../../lib/platform";
import { AnimatePresence, motion } from "motion/react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  ArrowDownToLine,
  ChevronDown,
  ChevronRight,
  Clock,
  Film,
  ListPlus,
  Music,
  Plus,
  Scissors,
  Sparkles,
  Wrench,
  X,
} from "lucide-react";
import {
  api,
  AudioFormat,
  Classification,
  createDefaultDownloadSpec,
  DownloadSpec,
  LibraryEntry,
  MediaMode,
  PlaylistEntryPreview,
  ProbeResult,
  ProbeScope,
  SponsorBlockAction,
  SubtitleMode,
  VideoProfile,
  WavBitDepth,
} from "../../lib/ipc";
import { formatBytes, formatDuration, formatKind } from "../../lib/format";
import { useAppStore } from "../../stores/appStore";
import { useQueueStore } from "../../stores/queueStore";
import { useSettingsStore } from "../../stores/settingsStore";
import {
  Button,
  Chip,
  Kbd,
  PathPreview,
  PlatformIcon,
  Popover,
  Segmented,
  Slider,
  SPRING_PANEL,
  SPRING_UI,
  StripeProgress,
  Toggle,
} from "../../ui/Primitives";
import { formatDecimal, formatNumber, useT } from "../../i18n";

const SPONSORBLOCK_CATS = [
  "sponsor",
  "selfpromo",
  "interaction",
  "intro",
  "outro",
  "preview",
  "filler",
  "music_offtopic",
] as const;

export const DownloadTab: React.FC<{
  urlInputRef: React.RefObject<HTMLInputElement | null>;
}> = ({ urlInputRef }) => {
  const t = useT();
  const stageElRef = useRef<HTMLDivElement | null>(null);
  const urlPlateRef = useRef<HTMLDivElement | null>(null);

  const settings = useSettingsStore((s) => s.settings);
  const reduceMotion = useSettingsStore((s) => s.settings.reduce_motion);
  const presets = useSettingsStore((s) => s.presets);
  const updateSettings = useSettingsStore((s) => s.updateSettings);
  const refreshPresets = useSettingsStore((s) => s.refreshPresets);

  const setStageRect = useAppStore((s) => s.setStageRect);
  const setUrlTargetCenter = useAppStore((s) => s.setUrlTargetCenter);
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const pendingUrlToLoad = useAppStore((s) => s.pendingUrlToLoad);
  const setPendingUrlToLoad = useAppStore((s) => s.setPendingUrlToLoad);
  const binsStatus = useAppStore((s) => s.binsStatus);

  const jobs = useQueueStore((s) => s.jobs);

  const [rawUrl, setRawUrl] = useState("");
  const [batchUrls, setBatchUrls] = useState<string[]>([]);
  const [classification, setClassification] = useState<Classification | null>(null);
  const [urlError, setUrlError] = useState<string | null>(null);

  const [probeScope, setProbeScope] = useState<ProbeScope>("single_video");
  const [probing, setProbing] = useState(false);
  const [probeResult, setProbeResult] = useState<ProbeResult | null>(null);
  const [probeError, setProbeError] = useState<{ message: string; hint?: string | null; code?: string } | null>(null);

  const [spec, setSpec] = useState<DownloadSpec>(() =>
    createDefaultDownloadSpec("video", settings.music_root, settings.video_root)
  );
  const [resolvedPathPreview, setResolvedPathPreview] = useState("");

  // Playlist selection & filtering state
  const [selectedIndices, setSelectedIndices] = useState<Set<number>>(new Set());
  const [rangeFrom, setRangeFrom] = useState("");
  const [rangeTo, setRangeTo] = useState("");
  const [hideShorts, setHideShorts] = useState(false);
  const [hideLives, setHideLives] = useState(false);
  const [minDurS, setMinDurS] = useState("");
  const [maxDurS, setMaxDurS] = useState("");

  // Collapsible sections & modals
  const [extrasOpen, setExtrasOpen] = useState(false);
  const [workshopOpen, setWorkshopOpen] = useState(false);
  const [presetName, setPresetName] = useState("");
  const [duplicateEntry, setDuplicateEntry] = useState<{
    entry: LibraryEntry;
    highPriority: boolean;
  } | null>(null);
  const [statusBanner, setStatusBanner] = useState<string | null>(null);

  const hasActiveLink =
    rawUrl.trim().length > 0 ||
    batchUrls.length > 0 ||
    probing ||
    probeResult !== null ||
    probeError !== null;

  // Measure Op-Art full-contrast stage rectangle (including smooth 60fps tracking during hero rise/center spring!)
  useEffect(() => {
    const syncRects = () => {
      if (stageElRef.current) {
        const r = stageElRef.current.getBoundingClientRect();
        setStageRect({ x: r.left, y: r.top, width: r.width, height: r.height });
      }
      if (urlPlateRef.current) {
        const u = urlPlateRef.current.getBoundingClientRect();
        setUrlTargetCenter(u.left + u.width / 2, u.top + u.height / 2);
      }
    };
    syncRects();

    // Track FLIP spring animation frames for 750ms whenever centered/active state changes
    let rafId = 0;
    const t0 = performance.now();
    const trackSpring = (now: number) => {
      syncRects();
      if (now - t0 < 750) {
        rafId = requestAnimationFrame(trackSpring);
      }
    };
    rafId = requestAnimationFrame(trackSpring);

    const ro = new ResizeObserver(syncRects);
    if (stageElRef.current) ro.observe(stageElRef.current);
    window.addEventListener("resize", syncRects);
    return () => {
      if (rafId) cancelAnimationFrame(rafId);
      ro.disconnect();
      window.removeEventListener("resize", syncRects);
    };
  }, [setStageRect, setUrlTargetCenter, hasActiveLink, probeResult, probing]);

  // Consume external URL paste/drop/re-download triggers
  useEffect(() => {
    if (pendingUrlToLoad) {
      handleUrlInput(pendingUrlToLoad);
      setPendingUrlToLoad(null);
    }
  }, [pendingUrlToLoad]);

  // Sync roots when settings finish loading if user hasn't typed a URL yet
  useEffect(() => {
    if (!rawUrl) {
      setSpec((prev) => ({
        ...prev,
        output_root: prev.mode === "audio" ? settings.music_root : settings.video_root,
        output_template:
          prev.mode === "audio" ? settings.audio_template : settings.video_template,
      }));
    }
  }, [settings.music_root, settings.video_root, settings.audio_template, settings.video_template]);

  const applyPlatformDefaults = (cls: Classification) => {
    const remembered = settings.last_spec_by_platform[cls.platform_id];
    const platformRule = settings.platform_rules.find(
      (r) => r.enabled && r.platform === cls.platform_id
    );
    if (remembered) {
      setSpec({
        ...remembered,
        output_root:
          platformRule?.custom_root ??
          (remembered.mode === "audio" ? settings.music_root : settings.video_root),
        output_template:
          platformRule?.template ??
          (remembered.mode === "audio" ? settings.audio_template : settings.video_template),
      });
    } else {
      const mode = cls.default_mode;
      const fresh = createDefaultDownloadSpec(mode, settings.music_root, settings.video_root);
      fresh.output_template =
        platformRule?.template ??
        (mode === "audio" ? settings.audio_template : settings.video_template);
      if (platformRule?.custom_root) {
        fresh.output_root = platformRule.custom_root;
      }
      setSpec(fresh);
    }
  };

  const triggerProbe = async (targetUrl: string, scope: ProbeScope) => {
    setProbing(true);
    setProbeError(null);
    try {
      const res = await api.probe(targetUrl, scope);
      setProbeResult(res);
      setClassification(res.classification);
      if (res.is_playlist) {
        setSelectedIndices(new Set(res.entries.map((e) => e.index)));
        setSpec((prev) => ({
          ...prev,
          playlist_numbering: true,
          output_template:
            prev.mode === "audio"
              ? settings.playlist_audio_template
              : settings.playlist_video_template,
        }));
      }
      if (res.is_upcoming) {
        setSpec((prev) => ({ ...prev, live_mode: "wait_for_premiere" }));
      }
      if (res.video_resolutions.length > 0) {
        setSpec((prev) => {
          if (
            prev.max_height !== null &&
            !res.video_resolutions.some((r) => r.height === prev.max_height)
          ) {
            const bestUnder1080 = res.video_resolutions.find((r) => r.height <= 1080);
            return { ...prev, max_height: bestUnder1080 ? bestUnder1080.height : null };
          }
          return prev;
        });
      }
    } catch (err: any) {
      setProbeResult(null);
      setProbeError({
        message: err?.message_pt || String(err),
        hint: err?.hint_pt ?? null,
        code: err?.code,
      });
    } finally {
      setProbing(false);
    }
  };

  const handleUrlInput = async (val: string) => {
    setRawUrl(val);
    setUrlError(null);
    setProbeError(null);
    setDuplicateEntry(null);
    setStatusBanner(null);

    const trimmed = val.trim();
    if (!trimmed) {
      setClassification(null);
      setProbeResult(null);
      setBatchUrls([]);
      return;
    }

    // Check if pasted text contains multiple URLs (Section 5.1)
    if (trimmed.includes("\n") || (trimmed.match(/https?:\/\//g)?.length ?? 0) > 1) {
      const urls = await api.extractUrls(trimmed);
      if (urls.length > 1) {
        setBatchUrls(urls);
        const firstCls = await api.classifyUrl(urls[0]).catch(() => null);
        if (firstCls) {
          setClassification(firstCls);
          applyPlatformDefaults(firstCls);
        }
        setProbeResult(null);
        return;
      } else if (urls.length === 1 && urls[0] !== trimmed) {
        setRawUrl(urls[0]);
      }
    }
    setBatchUrls([]);

    try {
      const cls = await api.classifyUrl(trimmed);
      setClassification(cls);
      applyPlatformDefaults(cls);
      setProbeScope("single_video");

      if (cls.is_drm) {
        setProbeResult(null);
        return;
      }
      triggerProbe(cls.normalized_url, "single_video");
    } catch (e: any) {
      setClassification(null);
      setProbeResult(null);
      if (trimmed.length > 8) {
        setUrlError(typeof e === "string" ? e : t.download.invalidLink);
      }
    }
  };

  const switchMode = (mode: MediaMode) => {
    const isPlaylist = probeResult?.is_playlist ?? false;
    setSpec((prev) => ({
      ...prev,
      mode,
      output_root: mode === "audio" ? settings.music_root : settings.video_root,
      output_template: isPlaylist
        ? mode === "audio"
          ? settings.playlist_audio_template
          : settings.playlist_video_template
        : mode === "audio"
        ? settings.audio_template
        : settings.video_template,
    }));
  };

  // Live resolved destination path preview (Section 10)
  useEffect(() => {
    let cancelled = false;
    const ext =
      spec.mode === "audio"
        ? spec.audio_format === "original"
          ? probeResult?.best_audio_acodec?.includes("mp4a")
            ? "m4a"
            : "opus"
          : spec.audio_format === "alac"
          ? "m4a"
          : spec.audio_format
        : spec.video_profile === "compatible"
        ? "mp4"
        : spec.video_profile === "max_quality"
        ? "mkv"
        : spec.custom_container;

    const uploadYear = probeResult?.upload_date?.slice(0, 4) || "2026";
    const uploadMonth = probeResult?.upload_date?.slice(5, 7) || "11";

    api
      .previewPath(spec.output_root, spec.output_template, {
        media_type: spec.mode === "audio" ? t.settings.previewSample.music : t.settings.previewSample.videos,
        platform: classification?.platform_name || "YouTube",
        creator: probeResult?.channel_or_artist || t.download.sampleCreator,
        playlist: probeResult?.playlist_title || null,
        index: 1,
        title: probeResult?.title || t.download.sampleTitle,
        track: probeResult?.title || null,
        id: probeResult?.id || "id",
        year: uploadYear,
        month: uploadMonth,
        ext,
      })
      .then((res) => {
        if (!cancelled) setResolvedPathPreview(res);
      });

    return () => {
      cancelled = true;
    };
  }, [spec, probeResult, classification, t]);

  // Filtered playlist items
  const filteredPlaylistEntries = useMemo(() => {
    if (!probeResult?.entries) return [];
    const minD = minDurS ? parseFloat(minDurS) : null;
    const maxD = maxDurS ? parseFloat(maxDurS) : null;
    return probeResult.entries.filter((item) => {
      if (hideShorts && item.is_short) return false;
      if (hideLives && item.is_live) return false;
      if (minD !== null && !Number.isNaN(minD) && (item.duration_s ?? 0) < minD) return false;
      if (maxD !== null && !Number.isNaN(maxD) && (item.duration_s ?? Infinity) > maxD) return false;
      return true;
    });
  }, [probeResult, hideShorts, hideLives, minDurS, maxDurS]);

  // Smart 4K VP9/AV1 vs MP4 H.264 compatibility warning (Section 7.3)
  const compatCodecWarning = useMemo(() => {
    if (
      spec.mode !== "video" ||
      spec.video_profile !== "compatible" ||
      !probeResult ||
      probeResult.video_resolutions.length === 0
    ) {
      return null;
    }
    const requestedH =
      spec.max_height ?? probeResult.video_resolutions[0]?.height ?? 1080;
    const matchRes = probeResult.video_resolutions.find((r) => r.height === requestedH);
    if (matchRes && !matchRes.has_h264 && !spec.workshop.video_reencode) {
      const lowerH264 =
        probeResult.video_resolutions.find((r) => r.has_h264 && r.height < requestedH)?.height ??
        null;
      return {
        requestedH,
        lowerH264,
        codecs: matchRes.vcodecs.join("/").toUpperCase(),
      };
    }
    return null;
  }, [spec, probeResult]);

  // Estimated output size
  const estimatedSizeBytes = useMemo(() => {
    if (!probeResult) return null;
    if (spec.mode === "audio") {
      return probeResult.best_audio_bytes;
    }
    const target = spec.max_height
      ? probeResult.video_resolutions.find((r) => r.height <= spec.max_height!)
      : probeResult.video_resolutions[0];
    return target?.estimated_total_bytes ?? null;
  }, [probeResult, spec.mode, spec.max_height]);

  const chooseCustomFolder = async () => {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      defaultPath: spec.output_root,
      title: t.download.pickFolderTitle,
    });
    if (typeof picked === "string" && picked.trim()) {
      setSpec((prev) => ({ ...prev, output_root: picked }));
    }
  };

  const handleSubmit = async (highPriority: boolean, forceDuplicate = false) => {
    setDuplicateEntry(null);
    setStatusBanner(null);

    if (batchUrls.length > 1) {
      const created = await api.enqueueBatch(batchUrls, spec, highPriority);
      setRawUrl("");
      setBatchUrls([]);
      setClassification(null);
      setStatusBanner(t.download.batchAdded(created.length));
      return;
    }

    const targetUrl = probeResult?.url || classification?.normalized_url || rawUrl.trim();
    if (!targetUrl || classification?.is_drm) return;

    if (probeResult?.is_playlist && !spec.concat_playlist) {
      const chosen: PlaylistEntryPreview[] = filteredPlaylistEntries.filter((e) =>
        selectedIndices.has(e.index)
      );
      if (chosen.length === 0) {
        setStatusBanner(t.download.selectPlaylistItem);
        return;
      }
      const created = await api.enqueuePlaylist(
        probeResult.playlist_title || probeResult.title,
        targetUrl,
        chosen,
        spec,
        highPriority
      );
      setStatusBanner(t.download.playlistAdded(created.length));
      return;
    }

    const outcome = await api.enqueue({
      url: targetUrl,
      title: probeResult?.title ?? null,
      channel_or_artist: probeResult?.channel_or_artist ?? null,
      thumb_url: probeResult?.thumb_url ?? null,
      duration_s: probeResult?.duration_s ?? null,
      estimated_bytes: estimatedSizeBytes,
      extractor_key: probeResult?.extractor_key ?? null,
      spec,
      high_priority: highPriority,
      force_duplicate: forceDuplicate,
    });

    if (outcome.status === "already_queued") {
      setStatusBanner(t.download.alreadyQueued);
    } else if (outcome.status === "already_in_library") {
      setDuplicateEntry({ entry: outcome.entry, highPriority });
    } else {
      setStatusBanner(
        highPriority ? t.download.startingNow : t.download.addedToQueue
      );
    }
  };

  const activeFooterJobs = useMemo(
    () =>
      jobs
        .filter((j) =>
          ["downloading", "postprocessing", "workshop", "probing", "waiting_premiere"].includes(
            j.state
          )
        )
        .slice(0, 3),
    [jobs]
  );

  const audioQualityNote = useMemo(() => {
    if (spec.mode !== "audio") return null;
    switch (spec.audio_format) {
      case "original":
        return t.download.audioNotes.original;
      case "mp3":
        return t.download.audioNotes.mp3;
      case "wav":
      case "aiff":
        return t.download.audioNotes.wav;
      case "flac":
      case "alac":
        return t.download.audioNotes.lossless;
      default:
        return null;
    }
  }, [spec.mode, spec.audio_format, t]);

  return (
    <div
      style={{
        height: "100%",
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
        position: "relative",
        zIndex: 2,
      }}
    >
      {/* Discreet yt2 migration notice */}
      {settings.migrated_from_yt2 && !settings.migration_banner_dismissed && (
        <div
          style={{
            margin: "10px 24px 0",
            padding: "8px 12px",
            borderRadius: 8,
            background: "var(--surface-1)",
            border: "1px solid var(--line)",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            fontSize: 12,
            color: "var(--text-2)",
          }}
        >
          <span>
            {t.download.migratedFromYt2}{" "}
            <strong className="font-mono-num" style={{ color: "var(--text)" }}>
              {settings.migrated_from_yt2}
            </strong>
          </span>
          <button
            type="button"
            onClick={() => updateSettings({ migration_banner_dismissed: true })}
            style={{
              background: "transparent",
              border: "none",
              color: "var(--text-3)",
              cursor: "pointer",
            }}
            aria-label={t.download.closeNotice}
          >
            <X size={14} />
          </button>
        </div>
      )}

      {/* Scrollable Main Area: vertically centered when empty; smoothly rises to top when a link enters! */}
      <div
        style={{
          flex: 1,
          minHeight: 0,
          overflowY: "auto",
          overflowX: "hidden",
          display: "flex",
          flexDirection: "column",
        }}
      >
        <div
          style={{
            minHeight: "100%",
            width: "100%",
            display: "flex",
            flexDirection: "column",
            justifyContent: hasActiveLink ? "flex-start" : "center",
            paddingBottom: hasActiveLink ? 20 : 28,
          }}
        >
          {/* Full-contrast Op-Art Stage around the hero URL input */}
          <motion.div
            ref={stageElRef}
            layout="position"
            transition={
              reduceMotion
                ? { duration: 0 }
                : { type: "spring", stiffness: 210, damping: 27, mass: 0.9 }
            }
            style={{
              width: "100%",
              padding: hasActiveLink ? "18px 24px 14px" : "42px 24px",
            }}
          >
            <div style={{ maxWidth: 900, width: "100%", margin: "0 auto" }}>
              <div
                ref={urlPlateRef}
                className="op-plate"
                style={{
                  height: 58,
                  width: "100%",
                  display: "flex",
                  alignItems: "center",
                  padding: "0 10px 0 14px",
                  gap: 10,
                  borderWidth: 2,
                  borderColor: urlError ? "var(--warn)" : "var(--line)",
                }}
              >
                <AnimatePresence mode="popLayout">
                  {classification && (
                    <motion.div
                      key={classification.platform_id + classification.kind}
                      initial={{ scale: 0.85, opacity: 0 }}
                      animate={{ scale: 1, opacity: 1 }}
                      exit={{ scale: 0.85, opacity: 0 }}
                      transition={SPRING_UI}
                      style={{
                        height: 34,
                        padding: "0 11px",
                        borderRadius: 999,
                        background: "var(--surface-2)",
                        border: "1px solid var(--line)",
                        display: "inline-flex",
                        alignItems: "center",
                        gap: 7,
                        flexShrink: 0,
                        maxWidth: 260,
                      }}
                    >
                      <PlatformIcon
                        slug={classification.icon_slug}
                        color={classification.brand_color}
                        size={15}
                      />
                      <span style={{ fontWeight: 700, fontSize: 12.5, whiteSpace: "nowrap" }}>
                        {classification.platform_name}
                      </span>
                      <span style={{ color: "var(--text-2)", fontSize: 12, whiteSpace: "nowrap" }}>
                        · {formatKind(classification.kind)}
                        {probeResult?.is_playlist && probeResult.playlist_count
                          ? ` · ${probeResult.playlist_count}`
                          : ""}
                      </span>
                    </motion.div>
                  )}
                </AnimatePresence>

                <input
                  ref={urlInputRef}
                  type="text"
                  value={rawUrl}
                  onChange={(e) => handleUrlInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && rawUrl.trim() && !classification?.is_drm) {
                      e.preventDefault();
                      handleSubmit(e.ctrlKey || e.metaKey);
                    }
                  }}
                  placeholder={t.download.urlPlaceholder}
                  aria-label={t.download.urlAria}
                  className="url-input"
                  style={{
                    flex: 1,
                    minWidth: 0,
                    height: "100%",
                    background: "transparent",
                    border: "none",
                    outline: "none",
                    color: "var(--text)",
                    fontSize: rawUrl.length > 48 ? 15.5 : 18,
                    fontFamily: "var(--font-sans)",
                    fontVariationSettings: '"wdth" 104, "wght" 560',
                    caretColor: "var(--accent)",
                    transition: "font-size 150ms ease",
                  }}
                />

                {rawUrl && (
                  <button
                    type="button"
                    onClick={() => handleUrlInput("")}
                    title={t.download.clearField}
                    style={{
                      width: 30,
                      height: 30,
                      borderRadius: 8,
                      border: "none",
                      background: "var(--surface-2)",
                      color: "var(--text-2)",
                      display: "inline-flex",
                      alignItems: "center",
                      justifyContent: "center",
                      cursor: "pointer",
                      flexShrink: 0,
                    }}
                  >
                    <X size={14} />
                  </button>
                )}
              </div>

              {/* Inline watch?v=...&list=... question (Section 1 & 14.2) */}
              {classification?.has_video_and_playlist && !classification.is_drm && (
                <div
                  className="op-panel"
                  style={{
                    marginTop: 10,
                    padding: "8px 14px",
                    display: "inline-flex",
                    alignItems: "center",
                    gap: 12,
                    flexWrap: "wrap",
                  }}
                >
                  <span style={{ fontSize: 13, color: "var(--text-2)" }}>
                    {t.download.insidePlaylist}
                  </span>
                  <Segmented<ProbeScope>
                    id="watch-playlist-scope"
                    value={probeScope}
                    onChange={(next) => {
                      setProbeScope(next);
                      triggerProbe(classification.normalized_url, next);
                    }}
                    options={[
                      { value: "single_video", label: t.download.onlyThisVideo },
                      {
                        value: "full_playlist",
                        label: t.download.wholePlaylist(probeResult?.playlist_count ?? null),
                      },
                    ]}
                  />
                </div>
              )}
            </div>
          </motion.div>

          {/* Main Content Area below Stage (only mounted when active so URL box centers cleanly when empty!) */}
          <AnimatePresence mode="popLayout">
            {hasActiveLink && (
              <motion.div
                key="active-download-panel"
                initial={reduceMotion ? { opacity: 1 } : { opacity: 0, y: 14 }}
                animate={{ opacity: 1, y: 0 }}
                exit={reduceMotion ? { opacity: 0 } : { opacity: 0, y: 10 }}
                transition={{ duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}
                style={{ width: "100%", padding: "2px 24px 0" }}
              >
                <div
                  style={{
                    maxWidth: 900,
                    width: "100%",
                    minWidth: 0,
                    margin: "0 auto",
                    display: "flex",
                    flexDirection: "column",
                    gap: 12,
                  }}
                >
          {/* Direct DRM notice without probing (Section 6) */}
          {classification?.is_drm && (
            <motion.div
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={SPRING_PANEL}
              className="op-panel"
              style={{
                padding: "16px 18px",
                borderColor: "var(--warn)",
                display: "flex",
                alignItems: "center",
                gap: 12,
              }}
            >
              <AlertTriangle size={20} color="var(--warn)" />
              <div>
                <div style={{ fontWeight: 700, color: "var(--warn)" }}>
                  {classification.drm_message}
                </div>
                <div style={{ fontSize: 12, color: "var(--text-2)", marginTop: 2 }}>
                  {t.download.drmBody}
                </div>
              </div>
            </motion.div>
          )}

          {/* Multi-URL batch paste banner */}
          {batchUrls.length > 1 && (
            <motion.div
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={SPRING_PANEL}
              className="op-panel"
              style={{
                padding: 18,
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                gap: 16,
              }}
            >
              <div>
                <div style={{ fontSize: 16, fontWeight: 700 }}>
                  {t.download.batchDetected(batchUrls.length)}
                </div>
                <div style={{ fontSize: 13, color: "var(--text-2)", marginTop: 4 }}>
                  {t.download.batchAllQueued(spec.mode === "audio" ? t.download.modeAudio : t.download.modeVideo)}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8 }}>
                <Button variant="secondary" onClick={() => switchMode(spec.mode === "audio" ? "video" : "audio")}>
                  {t.download.switchTo(spec.mode === "audio" ? t.download.modeVideo : t.download.modeAudio)}
                </Button>
                <Button variant="primary" size="lg" onClick={() => handleSubmit(false)}>
                  <ListPlus size={16} /> {t.download.addLinks(batchUrls.length)}
                </Button>
              </div>
            </motion.div>
          )}

          {/* Duplicate already-downloaded prompt */}
          {duplicateEntry && (
            <div
              className="op-panel"
              style={{
                padding: "12px 16px",
                borderColor: "var(--warn)",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                gap: 12,
              }}
            >
              <div style={{ fontSize: 13 }}>
                <strong style={{ color: "var(--warn)" }}>{t.download.duplicateTitle}</strong>{" "}
                {t.download.duplicateBody(duplicateEntry.entry.title, duplicateEntry.entry.format_badge)}
              </div>
              <div style={{ display: "flex", gap: 8 }}>
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => api.revealInExplorer(duplicateEntry.entry.primary_path)}
                >
                  {t.download.showInFolder}
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  onClick={() => handleSubmit(duplicateEntry.highPriority, true)}
                >
                  {t.download.downloadAgain}
                </Button>
              </div>
            </div>
          )}

          {/* Status feedback banner */}
          {statusBanner && (
            <div
              className="op-panel"
              style={{
                padding: "10px 14px",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                fontSize: 13,
              }}
            >
              <span>{statusBanner}</span>
              <Button size="sm" variant="ghost" onClick={() => setActiveTab("queue")}>
                {t.download.viewQueue}
              </Button>
            </div>
          )}

          {/* Probing skeleton ("Lendo o link...") */}
          {probing && (
            <div className="op-panel" style={{ padding: 16 }}>
              <div style={{ fontSize: 13, color: "var(--text-2)", marginBottom: 10 }}>
                {t.download.reading}
              </div>
              <div style={{ display: "grid", gridTemplateColumns: "220px minmax(0, 1fr)", gap: 16 }}>
                <div className="op-skeleton" style={{ height: 128, borderRadius: 8 }} />
                <div style={{ minWidth: 0, display: "flex", flexDirection: "column", gap: 10 }}>
                  <div className="op-skeleton" style={{ height: 22, width: "75%", borderRadius: 6 }} />
                  <div className="op-skeleton" style={{ height: 15, width: "45%", borderRadius: 6 }} />
                  <div className="op-skeleton" style={{ height: 34, width: "100%", borderRadius: 8, marginTop: 8 }} />
                  <div className="op-skeleton" style={{ height: 38, width: "60%", borderRadius: 8 }} />
                </div>
              </div>
            </div>
          )}

          {/* Probe Error with Human Recovery Action */}
          {probeError && !probing && (
            <div
              className="op-panel"
              style={{
                padding: 18,
                borderColor: "var(--warn)",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                gap: 16,
              }}
            >
              <div style={{ display: "flex", gap: 12, alignItems: "flex-start" }}>
                <AlertTriangle size={20} color="var(--warn)" style={{ marginTop: 2, flexShrink: 0 }} />
                <div>
                  <div style={{ fontWeight: 700, color: "var(--warn)" }}>{probeError.message}</div>
                  {probeError.hint && (
                    <div style={{ fontSize: 12, color: "var(--text-2)", marginTop: 4 }}>
                      {probeError.hint}
                    </div>
                  )}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
                {(probeError.code === "auth_required" || probeError.code === "bot_check") && (
                  <Button
                    variant="warn"
                    onClick={() => setActiveTab("settings", "accounts")}
                  >
                    {t.queue.connectAccount}
                  </Button>
                )}
                {(probeError.code === "extractor_broken" || probeError.code === "unsupported_url") && (
                  <Button
                    variant="warn"
                    onClick={async () => {
                      await api.updateYtdlp();
                      if (classification) triggerProbe(classification.normalized_url, probeScope);
                    }}
                  >
                    {t.queue.updateYtdlpRetry}
                  </Button>
                )}
                <Button
                  variant="secondary"
                  onClick={() =>
                    classification && triggerProbe(classification.normalized_url, probeScope)
                  }
                >
                  {t.queue.retry}
                </Button>
              </div>
            </div>
          )}

          {/* Main Preview + 3-Level Selector Card */}
          {probeResult && !probing && (
            <motion.div
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              transition={SPRING_PANEL}
              className="op-panel"
              style={{ padding: 18, display: "flex", flexDirection: "column", gap: 16 }}
            >
              {/* Preset Chips Row */}
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  flexWrap: "wrap",
                  paddingBottom: 12,
                  borderBottom: "1px solid var(--line)",
                }}
              >
                <span style={{ fontSize: 12, color: "var(--text-3)", marginRight: 4 }}>
                  {t.download.presetsLabel}
                </span>
                {presets.map((p) => (
                  <Chip
                    key={p.id}
                    title={p.description}
                    onClick={() =>
                      setSpec({
                        ...p.spec,
                        output_root:
                          p.spec.mode === "audio" ? settings.music_root : settings.video_root,
                      })
                    }
                  >
                    {p.name}
                  </Chip>
                ))}

                <Popover.Root>
                  <Popover.Trigger asChild>
                    <Chip style={{ borderStyle: "dashed" }}>
                      <Plus size={12} /> {t.download.saveCurrent}
                    </Chip>
                  </Popover.Trigger>
                  <Popover.Portal>
                    <Popover.Content
                      sideOffset={6}
                      className="op-panel"
                      style={{ padding: 12, width: 260, zIndex: 60 }}
                    >
                      <div style={{ fontSize: 12, fontWeight: 700, marginBottom: 8 }}>
                        {t.download.savePresetTitle}
                      </div>
                      <input
                        className="op-input"
                        placeholder={t.download.presetPlaceholder}
                        value={presetName}
                        onChange={(e) => setPresetName(e.target.value)}
                        style={{ width: "100%", height: 32, padding: "0 10px", fontSize: 12 }}
                      />
                      <div style={{ display: "flex", justifyContent: "flex-end", marginTop: 8 }}>
                        <Button
                          size="sm"
                          variant="primary"
                          disabled={!presetName.trim()}
                          onClick={async () => {
                            await api.saveCustomPreset(presetName.trim(), t.download.customPresetDesc, spec);
                            setPresetName("");
                            await refreshPresets();
                          }}
                        >
                          {t.download.save}
                        </Button>
                      </div>
                    </Popover.Content>
                  </Popover.Portal>
                </Popover.Root>
              </div>

              {/* 2-Column Media Info + Selector (strictly bounded with minmax(0, 1fr) + minWidth: 0!) */}
              <div
                style={{
                  display: "grid",
                  gridTemplateColumns: "220px minmax(0, 1fr)",
                  gap: 18,
                  alignItems: "start",
                  width: "100%",
                  minWidth: 0,
                }}
              >
                {/* Left Column: Cover + Metadata */}
                <div style={{ minWidth: 0, display: "flex", flexDirection: "column", gap: 10 }}>
                  <div
                    style={{
                      width: "100%",
                      height:
                        spec.mode === "audio"
                          ? 180
                          : probeResult.is_portrait
                          ? 176
                          : 124,
                      borderRadius: 8,
                      overflow: "hidden",
                      background: "var(--surface-2)",
                      border: "1px solid var(--line)",
                      position: "relative",
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "center",
                    }}
                  >
                    {probeResult.thumb_url ? (
                      <img
                        src={probeResult.thumb_url}
                        alt={probeResult.title}
                        style={{
                          width: "100%",
                          height: "100%",
                          objectFit:
                            spec.mode !== "audio" && probeResult.is_portrait
                              ? "contain"
                              : "cover",
                        }}
                      />
                    ) : (
                      <div
                        style={{
                          width: "100%",
                          height: "100%",
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "center",
                          color: "var(--text-3)",
                        }}
                      >
                        {spec.mode === "audio" ? <Music size={28} /> : <Film size={28} />}
                      </div>
                    )}
                    {probeResult.duration_s && (
                      <span
                        className="font-mono-num"
                        style={{
                          position: "absolute",
                          right: 8,
                          bottom: 8,
                          padding: "2px 6px",
                          borderRadius: 4,
                          background: "var(--bg)",
                          border: "1px solid var(--line)",
                          fontSize: 11,
                          color: "var(--text)",
                        }}
                      >
                        {formatDuration(probeResult.duration_s)}
                      </span>
                    )}
                  </div>

                  <div style={{ minWidth: 0 }}>
                    <div
                      className="selectable"
                      title={probeResult.title}
                      style={{
                        fontSize: 14.5,
                        fontWeight: 700,
                        lineHeight: 1.32,
                        display: "-webkit-box",
                        WebkitLineClamp: 3,
                        WebkitBoxOrient: "vertical",
                        overflow: "hidden",
                        overflowWrap: "anywhere",
                        wordBreak: "break-word",
                      }}
                    >
                      {probeResult.title}
                    </div>
                    <div
                      style={{
                        fontSize: 12.5,
                        color: "var(--text-2)",
                        marginTop: 4,
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                      }}
                    >
                      {probeResult.channel_or_artist}
                      {probeResult.upload_date ? ` · ${probeResult.upload_date}` : ""}
                    </div>
                    {estimatedSizeBytes && (
                      <div
                        className="font-mono-num"
                        style={{ fontSize: 11.5, color: "var(--text-3)", marginTop: 3 }}
                      >
                        {t.download.estimatedSize(formatBytes(estimatedSizeBytes))}
                      </div>
                    )}
                  </div>
                </div>

                {/* Right Column: Level 1 + Level 2 (Extras) + Level 3 (Oficina) + Destination + Actions */}
                <div style={{ minWidth: 0, maxWidth: "100%", display: "flex", flexDirection: "column", gap: 12 }}>
                  {/* Level 1: Mode + Format + Quality */}
                  <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", flexWrap: "wrap", gap: 10 }}>
                    <Segmented<MediaMode>
                      id="media-mode"
                      value={spec.mode}
                      onChange={switchMode}
                      options={[
                        {
                          value: "video",
                          label: (
                            <>
                              <Film size={14} /> {t.download.modeVideo}
                            </>
                          ),
                        },
                        {
                          value: "audio",
                          label: (
                            <>
                              <Music size={14} /> {t.download.modeAudio}
                            </>
                          ),
                        },
                      ]}
                    />

                    {/* Live / Upcoming Premiere Options */}
                    {(probeResult.is_live || probeResult.is_upcoming) && (
                      <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                        <Clock size={14} color="var(--accent-text)" />
                        {probeResult.is_upcoming ? (
                          <Chip
                            active={spec.live_mode === "wait_for_premiere"}
                            onClick={() => setSpec({ ...spec, live_mode: "wait_for_premiere" })}
                          >
                            {t.download.waitPremiere}
                          </Chip>
                        ) : (
                          <>
                            <Chip
                              active={spec.live_mode === "from_start"}
                              onClick={() => setSpec({ ...spec, live_mode: "from_start" })}
                            >
                              {t.download.fromStart}
                            </Chip>
                            <Chip
                              active={spec.live_mode === "from_now"}
                              onClick={() => setSpec({ ...spec, live_mode: "from_now" })}
                            >
                              {t.download.fromNow}
                            </Chip>
                          </>
                        )}
                      </div>
                    )}
                  </div>

                  {spec.mode === "audio" ? (
                    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                      <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                        {(
                          [
                            ["original", t.download.formatOriginal],
                            ["mp3", "MP3"],
                            ["m4a", "M4A (AAC)"],
                            ["opus", "Opus"],
                            ["flac", "FLAC"],
                            ["alac", "ALAC"],
                            ["wav", "WAV"],
                            ["aiff", "AIFF"],
                          ] as [AudioFormat, string][]
                        ).map(([fmt, label]) => (
                          <Chip
                            key={fmt}
                            active={spec.audio_format === fmt}
                            onClick={() => setSpec({ ...spec, audio_format: fmt })}
                          >
                            {label}
                          </Chip>
                        ))}
                      </div>

                      {spec.audio_format === "mp3" && (
                        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                          <span style={{ fontSize: 12, color: "var(--text-2)" }}>{t.download.quality}</span>
                          <Chip
                            active={spec.mp3_quality === "v0"}
                            onClick={() => setSpec({ ...spec, mp3_quality: "v0" })}
                          >
                            {t.download.mp3Recommended}
                          </Chip>
                          <Chip
                            active={spec.mp3_quality === "cbr320"}
                            onClick={() => setSpec({ ...spec, mp3_quality: "cbr320" })}
                          >
                            320 kbps CBR
                          </Chip>
                        </div>
                      )}

                      {(spec.audio_format === "wav" || spec.audio_format === "aiff") && (
                        <div style={{ display: "flex", gap: 10, flexWrap: "wrap", alignItems: "center" }}>
                          <span style={{ fontSize: 12, color: "var(--text-2)" }}>{t.download.bitDepth}</span>
                          {(
                            [
                              ["s16", "16-bit"],
                              ["s24", "24-bit"],
                              ...(spec.audio_format === "wav" ? ([["f32", "32-bit float"]] as const) : []),
                            ] as [WavBitDepth, string][]
                          ).map(([bd, lbl]) => (
                            <Chip
                              key={bd}
                              active={spec.wav_bit_depth === bd}
                              onClick={() => setSpec({ ...spec, wav_bit_depth: bd })}
                            >
                              {lbl}
                            </Chip>
                          ))}
                          <span style={{ fontSize: 12, color: "var(--text-2)", marginLeft: 6 }}>{t.download.sampleRate}</span>
                          {[44100, 48000, 96000].map((hz) => (
                            <Chip
                              key={hz}
                              active={spec.sample_rate_hz === hz}
                              onClick={() => setSpec({ ...spec, sample_rate_hz: hz })}
                            >
                              {hz % 1000 === 0 ? hz / 1000 : formatDecimal(hz / 1000, 1)} kHz
                            </Chip>
                          ))}
                        </div>
                      )}

                      {audioQualityNote && (
                        <div style={{ fontSize: 12, color: "var(--text-2)" }}>{audioQualityNote}</div>
                      )}
                    </div>
                  ) : (
                    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                      {/* Profile selector */}
                      <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
                        <span style={{ fontSize: 12, color: "var(--text-2)" }}>{t.download.profile}</span>
                        {(
                          [
                            ["compatible", t.download.profileCompatible],
                            ["max_quality", t.download.profileMax],
                            ["custom", t.download.profileCustom],
                          ] as [VideoProfile, string][]
                        ).map(([prof, lbl]) => (
                          <Chip
                            key={prof}
                            active={spec.video_profile === prof}
                            onClick={() => setSpec({ ...spec, video_profile: prof })}
                          >
                            {lbl}
                          </Chip>
                        ))}
                      </div>

                      {spec.video_profile === "custom" && (
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
                          <span style={{ fontSize: 12, color: "var(--text-2)" }}>{t.download.container}</span>
                          {(["mp4", "mkv", "webm", "mov"] as const).map((ct) => (
                            <Chip
                              key={ct}
                              active={spec.custom_container === ct}
                              onClick={() => setSpec({ ...spec, custom_container: ct })}
                            >
                              {ct.toUpperCase()}
                            </Chip>
                          ))}
                        </div>
                      )}

                      {/* Real probed resolution chips */}
                      {probeResult.video_resolutions.length > 0 && (
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
                          <span style={{ fontSize: 12, color: "var(--text-2)" }}>{t.download.resolution}</span>
                          <Chip
                            active={spec.max_height === null}
                            onClick={() => setSpec({ ...spec, max_height: null })}
                          >
                            {t.download.bestAvailable}
                          </Chip>
                          {Array.from(
                            new Map(probeResult.video_resolutions.map((r) => [r.height, r])).values()
                          ).map((res) => (
                            <Chip
                              key={res.height}
                              active={spec.max_height === res.height}
                              onClick={() => setSpec({ ...spec, max_height: res.height })}
                            >
                              {res.label}
                              {res.has_hdr && (
                                <span
                                  style={{
                                    fontSize: 10,
                                    padding: "0 4px",
                                    borderRadius: 4,
                                    background: "var(--bg)",
                                    color: "var(--accent-text)",
                                  }}
                                >
                                  HDR
                                </span>
                              )}
                            </Chip>
                          ))}
                        </div>
                      )}

                      {/* Multiple Audio Languages */}
                      {probeResult.audio_languages.length > 1 && (
                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
                          <span style={{ fontSize: 12, color: "var(--text-2)" }}>{t.download.audioTrack}</span>
                          <Chip
                            active={!spec.audio_language || spec.audio_language === "original"}
                            onClick={() => setSpec({ ...spec, audio_language: null })}
                          >
                            {t.download.formatOriginal}
                          </Chip>
                          {probeResult.audio_languages.map((lang) => (
                            <Chip
                              key={lang}
                              active={spec.audio_language === lang}
                              onClick={() => setSpec({ ...spec, audio_language: lang })}
                            >
                              {lang}
                            </Chip>
                          ))}
                        </div>
                      )}

                      {/* Smart 4K VP9/AV1 vs MP4 H.264 warning (Section 7.3) */}
                      {compatCodecWarning && (
                        <div
                          style={{
                            padding: "10px 12px",
                            borderRadius: 8,
                            border: "1px solid var(--warn)",
                            background: "var(--surface-2)",
                            fontSize: 12,
                            display: "flex",
                            flexDirection: "column",
                            gap: 8,
                            maxWidth: "100%",
                            minWidth: 0,
                          }}
                        >
                          <span style={{ color: "var(--warn)", fontWeight: 600 }}>
                            {compatCodecWarning.lowerH264
                              ? t.download.codecWarnLower(
                                  compatCodecWarning.requestedH,
                                  compatCodecWarning.codecs,
                                  compatCodecWarning.lowerH264
                                )
                              : t.download.codecWarn(compatCodecWarning.requestedH, compatCodecWarning.codecs)}
                          </span>
                          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                            {compatCodecWarning.lowerH264 && (
                              <Button
                                size="sm"
                                variant="secondary"
                                onClick={() =>
                                  setSpec({ ...spec, max_height: compatCodecWarning.lowerH264 })
                                }
                              >
                                {t.download.useH264(compatCodecWarning.lowerH264)}
                              </Button>
                            )}
                            <Button
                              size="sm"
                              variant="secondary"
                              onClick={() =>
                                setSpec({ ...spec, video_profile: "max_quality" })
                              }
                            >
                              {t.download.keepMkv(compatCodecWarning.requestedH)}
                            </Button>
                            <Button
                              size="sm"
                              variant="secondary"
                              onClick={() =>
                                setSpec({
                                  ...spec,
                                  video_profile: "max_quality",
                                  workshop: {
                                    ...spec.workshop,
                                    video_reencode: {
                                      codec: "h264",
                                      quality: "high",
                                      prefer_gpu: true,
                                    },
                                  },
                                })
                              }
                            >
                              {t.download.reencodeH264}
                            </Button>
                          </div>
                        </div>
                      )}
                    </div>
                  )}

                  {/* Level 2 (Extras) & Level 3 (Oficina) Collapsible Toggles */}
                  <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setExtrasOpen((v) => !v)}
                    >
                      {extrasOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                      <Scissors size={13} /> {t.download.extrasToggle}
                    </Button>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setWorkshopOpen((v) => !v)}
                    >
                      {workshopOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                      <Wrench size={13} /> {t.download.workshopToggle}
                    </Button>
                  </div>

                  {/* Level 2: Extras Panel */}
                  {extrasOpen && (
                    <div
                      style={{
                        padding: 12,
                        borderRadius: 8,
                        background: "var(--surface-2)",
                        border: "1px solid var(--line)",
                        display: "flex",
                        flexDirection: "column",
                        gap: 12,
                      }}
                    >
                      {spec.mode === "audio" ? (
                        <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
                          <Toggle
                            checked={spec.embed_square_cover}
                            disabled={spec.audio_format === "wav" || spec.audio_format === "aiff"}
                            onChange={(v) => setSpec({ ...spec, embed_square_cover: v })}
                            label={t.download.squareCover}
                          />
                          <Toggle
                            checked={spec.parse_artist_title}
                            onChange={(v) => setSpec({ ...spec, parse_artist_title: v })}
                            label={t.download.splitArtistTitle}
                          />
                          <Toggle
                            checked={spec.clean_music_title}
                            onChange={(v) => setSpec({ ...spec, clean_music_title: v })}
                            label={t.download.cleanTitle}
                          />
                        </div>
                      ) : (
                        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                          <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                            <span style={{ fontSize: 12, color: "var(--text-2)" }}>{t.download.subtitles}</span>
                            {(
                              [
                                ["off", t.download.subsOff],
                                ["embed", t.download.subsEmbed],
                                ["sidecar_srt", t.download.subsSidecar],
                              ] as [SubtitleMode, string][]
                            ).map(([m, l]) => (
                              <Chip
                                key={m}
                                active={spec.subtitle_mode === m}
                                onClick={() => setSpec({ ...spec, subtitle_mode: m })}
                              >
                                {l}
                              </Chip>
                            ))}
                            {spec.subtitle_mode !== "off" && (
                              <Toggle
                                checked={spec.include_auto_subs}
                                onChange={(v) => setSpec({ ...spec, include_auto_subs: v })}
                                label={t.download.includeAutoSubs}
                              />
                            )}
                          </div>
                          <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
                            <Toggle
                              checked={spec.embed_chapters}
                              onChange={(v) => setSpec({ ...spec, embed_chapters: v })}
                              label={t.download.embedChapters}
                            />
                            {probeResult.chapters.length > 0 && (
                              <Toggle
                                checked={spec.split_chapters}
                                onChange={(v) => setSpec({ ...spec, split_chapters: v })}
                                label={t.download.splitChapters(probeResult.chapters.length)}
                              />
                            )}
                          </div>
                        </div>
                      )}

                      {/* Section Cut */}
                      {probeResult.duration_s && probeResult.duration_s > 2 && (
                        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                            <span style={{ fontSize: 12, color: "var(--text-2)" }}>
                              {t.download.cutSection}{" "}
                              <strong className="font-mono-num" style={{ color: "var(--text)" }}>
                                {formatDuration(spec.section_start_s ?? 0)} —{" "}
                                {formatDuration(spec.section_end_s ?? probeResult.duration_s)}
                              </strong>
                            </span>
                            <div style={{ display: "flex", gap: 10, alignItems: "center" }}>
                              {(spec.section_start_s !== null || spec.section_end_s !== null) && (
                                <>
                                  <Toggle
                                    checked={spec.force_keyframes_at_cuts}
                                    onChange={(v) =>
                                      setSpec({ ...spec, force_keyframes_at_cuts: v })
                                    }
                                    label={t.download.preciseCut}
                                  />
                                  <Button
                                    size="sm"
                                    variant="ghost"
                                    onClick={() =>
                                      setSpec({
                                        ...spec,
                                        section_start_s: null,
                                        section_end_s: null,
                                      })
                                    }
                                  >
                                    {t.download.wholeMedia}
                                  </Button>
                                </>
                              )}
                            </div>
                          </div>
                          <Slider
                            min={0}
                            max={Math.round(probeResult.duration_s)}
                            step={1}
                            value={[
                              Math.round(spec.section_start_s ?? 0),
                              Math.round(spec.section_end_s ?? probeResult.duration_s),
                            ]}
                            onValueChange={([a, b]) =>
                              setSpec({
                                ...spec,
                                section_start_s: a > 0 ? a : null,
                                section_end_s: b < Math.round(probeResult.duration_s!) ? b : null,
                              })
                            }
                            ariaLabel={t.download.cutRangeAria}
                          />
                        </div>
                      )}

                      {/* SponsorBlock */}
                      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                        <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
                          <span style={{ fontSize: 12, color: "var(--text-2)" }}>SponsorBlock:</span>
                          {(
                            [
                              ["off", t.download.sponsorOff],
                              ["remove", t.download.sponsorRemove],
                              ["mark", t.download.sponsorMark],
                            ] as [SponsorBlockAction, string][]
                          ).map(([act, lbl]) => (
                            <Chip
                              key={act}
                              active={spec.sponsorblock_action === act}
                              onClick={() => setSpec({ ...spec, sponsorblock_action: act })}
                            >
                              {lbl}
                            </Chip>
                          ))}
                          <Chip
                            onClick={() =>
                              setSpec({
                                ...spec,
                                sponsorblock_action: "remove",
                                sponsorblock_categories: ["music_offtopic"],
                              })
                            }
                          >
                            <Sparkles size={12} /> {t.download.sponsorMusicClip}
                          </Chip>
                        </div>
                        {spec.sponsorblock_action !== "off" && (
                          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                            {SPONSORBLOCK_CATS.map((cat) => {
                              const on = spec.sponsorblock_categories.includes(cat);
                              return (
                                <Chip
                                  key={cat}
                                  active={on}
                                  onClick={() =>
                                    setSpec({
                                      ...spec,
                                      sponsorblock_categories: on
                                        ? spec.sponsorblock_categories.filter((c) => c !== cat)
                                        : [...spec.sponsorblock_categories, cat],
                                    })
                                  }
                                >
                                  {t.download.sponsorCats[cat]}
                                </Chip>
                              );
                            })}
                          </div>
                        )}
                      </div>

                      {probeResult.is_playlist && (
                        <Toggle
                          checked={spec.concat_playlist}
                          onChange={(v) => setSpec({ ...spec, concat_playlist: v })}
                          label={t.download.concatPlaylist}
                        />
                      )}
                    </div>
                  )}

                  {/* Level 3: FFmpeg Workshop Panel */}
                  {workshopOpen && (
                    <div
                      style={{
                        padding: 12,
                        borderRadius: 8,
                        background: "var(--surface-2)",
                        border: "1px solid var(--line)",
                        display: "flex",
                        flexDirection: "column",
                        gap: 10,
                      }}
                    >
                      <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
                        <span style={{ fontSize: 12, color: "var(--text-2)" }}>
                          {t.download.normalizeVolume}
                        </span>
                        {[null, -14, -16, -23].map((lufs) => (
                          <Chip
                            key={String(lufs)}
                            active={spec.workshop.loudnorm_lufs === lufs}
                            onClick={() =>
                              setSpec({
                                ...spec,
                                workshop: { ...spec.workshop, loudnorm_lufs: lufs },
                              })
                            }
                          >
                            {lufs === null ? t.download.formatOriginal : `${lufs} LUFS`}
                          </Chip>
                        ))}
                      </div>

                      <div style={{ display: "flex", gap: 14, flexWrap: "wrap", alignItems: "center" }}>
                        {/* Trimming only the soundtrack would push a video out of sync. */}
                        {spec.mode === "audio" && (
                          <Toggle
                            checked={spec.workshop.trim_silence}
                            onChange={(v) =>
                              setSpec({
                                ...spec,
                                workshop: { ...spec.workshop, trim_silence: v },
                              })
                            }
                            label={t.download.trimSilence}
                          />
                        )}
                        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                          <span style={{ fontSize: 12, color: "var(--text-2)" }}>{t.download.speed}</span>
                          {[null, 0.75, 1.25, 1.5, 2.0].map((spd) => (
                            <Chip
                              key={String(spd)}
                              active={spec.workshop.speed_factor === spd}
                              onClick={() =>
                                setSpec({
                                  ...spec,
                                  workshop: { ...spec.workshop, speed_factor: spd },
                                })
                              }
                            >
                              {spd === null ? "1x" : `${formatNumber(spd, 2)}x`}
                            </Chip>
                          ))}
                        </div>
                        {binsStatus.capabilities.has_rubberband && (
                          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                            <span style={{ fontSize: 12, color: "var(--text-2)" }}>{t.download.pitch}</span>
                            {[-2, -1, null, 1, 2].map((st) => (
                              <Chip
                                key={String(st)}
                                active={spec.workshop.pitch_semitones === st}
                                onClick={() =>
                                  setSpec({
                                    ...spec,
                                    workshop: { ...spec.workshop, pitch_semitones: st },
                                  })
                                }
                              >
                                {st === null ? "0 st" : st > 0 ? `+${st}` : `${st}`}
                              </Chip>
                            ))}
                          </div>
                        )}
                      </div>

                      {spec.mode === "video" && (
                        <div style={{ display: "flex", gap: 10, flexWrap: "wrap", alignItems: "center" }}>
                          <span style={{ fontSize: 12, color: "var(--text-2)" }}>
                            {t.download.compressForMessaging}
                          </span>
                          {[null, 10, 16, 25, 50].map((mb) => (
                            <Chip
                              key={String(mb)}
                              active={spec.workshop.target_size_mb === mb}
                              onClick={() =>
                                setSpec({
                                  ...spec,
                                  workshop: { ...spec.workshop, target_size_mb: mb },
                                })
                              }
                            >
                              {mb === null ? t.download.noTarget : `${mb} MB`}
                            </Chip>
                          ))}
                          <Toggle
                            checked={spec.workshop.make_gif !== null}
                            onChange={(v) =>
                              setSpec({
                                ...spec,
                                workshop: {
                                  ...spec.workshop,
                                  make_gif: v ? { fps: 15, width: 480 } : null,
                                },
                              })
                            }
                            label={t.download.makeGif}
                          />
                          {binsStatus.capabilities.has_libass && (
                            <Toggle
                              checked={spec.workshop.burn_subtitles}
                              onChange={(v) =>
                                setSpec({
                                  ...spec,
                                  workshop: { ...spec.workshop, burn_subtitles: v },
                                })
                              }
                              label={t.download.burnSubs}
                            />
                          )}
                        </div>
                      )}

                      {binsStatus.capabilities.has_whisper && (
                        <Toggle
                          checked={spec.workshop.whisper !== null}
                          onChange={(v) =>
                            setSpec({
                              ...spec,
                              workshop: {
                                ...spec.workshop,
                                whisper: v ? { model: "base", language: "auto" } : null,
                              },
                            })
                          }
                          label={t.download.whisper}
                        />
                      )}
                    </div>
                  )}

                  {/* Destination Path + Recent Folders Popover */}
                  <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <PathPreview path={resolvedPathPreview} onClick={chooseCustomFolder} />
                    </div>
                    {settings.recent_folders.length > 0 && (
                      <Popover.Root>
                        <Popover.Trigger asChild>
                          <Button size="sm" variant="secondary" title={t.download.recentFolders}>
                            {t.download.recent}
                          </Button>
                        </Popover.Trigger>
                        <Popover.Portal>
                          <Popover.Content
                            sideOffset={6}
                            className="op-panel"
                            style={{ padding: 6, width: 320, zIndex: 60 }}
                          >
                            {settings.recent_folders.map((folder) => (
                              <button
                                key={folder}
                                type="button"
                                onClick={() => setSpec({ ...spec, output_root: folder })}
                                className="font-mono-num"
                                style={{
                                  width: "100%",
                                  textAlign: "left",
                                  padding: "6px 8px",
                                  fontSize: 11,
                                  background: "transparent",
                                  border: "none",
                                  color: "var(--text)",
                                  borderRadius: 6,
                                  cursor: "pointer",
                                  overflow: "hidden",
                                  textOverflow: "ellipsis",
                                  whiteSpace: "nowrap",
                                }}
                              >
                                {folder}
                              </button>
                            ))}
                          </Popover.Content>
                        </Popover.Portal>
                      </Popover.Root>
                    )}
                  </div>

                  {/* Primary Action Buttons */}
                  <div style={{ display: "flex", gap: 10, justifyContent: "flex-end", alignItems: "center" }}>
                    <span style={{ fontSize: 11, color: "var(--text-3)", marginRight: "auto" }}>
                      <Kbd>Enter</Kbd> {t.download.kbdQueue} · <Kbd>{MOD_KEY}+Enter</Kbd> {t.download.kbdNow}
                    </span>
                    <Button
                      variant="secondary"
                      size="lg"
                      onClick={() => handleSubmit(false)}
                    >
                      <ListPlus size={16} /> {t.download.addToQueue}
                    </Button>
                    <Button
                      variant="primary"
                      size="lg"
                      onClick={() => handleSubmit(true)}
                    >
                      <ArrowDownToLine size={16} /> {t.download.downloadNow}
                    </Button>
                  </div>
                </div>
              </div>

              {/* Playlist Items Selector (Section 9.3) */}
              {probeResult.is_playlist && probeResult.entries.length > 0 && (
                <div
                  style={{
                    borderTop: "1px solid var(--line)",
                    paddingTop: 14,
                    display: "flex",
                    flexDirection: "column",
                    gap: 10,
                  }}
                >
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      flexWrap: "wrap",
                      gap: 10,
                    }}
                  >
                    <div style={{ fontSize: 13, fontWeight: 700 }}>
                      {t.download.selectedItems(selectedIndices.size, filteredPlaylistEntries.length)}
                    </div>
                    <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() =>
                          setSelectedIndices(new Set(filteredPlaylistEntries.map((e) => e.index)))
                        }
                      >
                        {t.download.selectAll}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => setSelectedIndices(new Set())}
                      >
                        {t.download.selectNone}
                      </Button>
                      <input
                        className="op-input font-mono-num"
                        placeholder={t.download.rangeFrom}
                        value={rangeFrom}
                        onChange={(e) => setRangeFrom(e.target.value)}
                        style={{ width: 58, height: 28, padding: "0 6px", fontSize: 12 }}
                      />
                      <input
                        className="op-input font-mono-num"
                        placeholder={t.download.rangeTo}
                        value={rangeTo}
                        onChange={(e) => setRangeTo(e.target.value)}
                        style={{ width: 58, height: 28, padding: "0 6px", fontSize: 12 }}
                      />
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => {
                          const a = parseInt(rangeFrom, 10) || 1;
                          const b = parseInt(rangeTo, 10) || probeResult.entries.length;
                          setSelectedIndices(
                            new Set(
                              filteredPlaylistEntries
                                .filter((e) => e.index >= Math.min(a, b) && e.index <= Math.max(a, b))
                                .map((e) => e.index)
                            )
                          );
                        }}
                      >
                        {t.download.applyRange}
                      </Button>
                      <Toggle
                        checked={hideShorts}
                        onChange={setHideShorts}
                        label={t.download.skipShorts}
                      />
                      <Toggle
                        checked={hideLives}
                        onChange={setHideLives}
                        label={t.download.skipLives}
                      />
                      <Toggle
                        checked={spec.playlist_numbering}
                        onChange={(v) => setSpec({ ...spec, playlist_numbering: v })}
                        label={t.download.numberItems}
                      />
                    </div>
                  </div>

                  <div
                    style={{
                      maxHeight: 220,
                      overflowY: "auto",
                      border: "1px solid var(--line)",
                      borderRadius: 8,
                      background: "var(--surface-2)",
                    }}
                  >
                    {filteredPlaylistEntries.map((item) => {
                      const checked = selectedIndices.has(item.index);
                      return (
                        <label
                          key={item.index}
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: 10,
                            padding: "6px 12px",
                            borderBottom: "1px solid var(--line)",
                            cursor: "pointer",
                            fontSize: 13,
                          }}
                        >
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={() => {
                              const next = new Set(selectedIndices);
                              if (checked) next.delete(item.index);
                              else next.add(item.index);
                              setSelectedIndices(next);
                            }}
                          />
                          <span className="font-mono-num" style={{ color: "var(--text-3)", width: 28 }}>
                            #{item.index}
                          </span>
                          <span
                            style={{
                              flex: 1,
                              overflow: "hidden",
                              textOverflow: "ellipsis",
                              whiteSpace: "nowrap",
                            }}
                          >
                            {item.title}
                          </span>
                          <span className="font-mono-num" style={{ fontSize: 12, color: "var(--text-2)" }}>
                            {formatDuration(item.duration_s)}
                          </span>
                        </label>
                      );
                    })}
                  </div>
                </div>
              )}
            </motion.div>
          )}
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </div>

      {/* Footer Strip: "Em andamento" (up to 3 active jobs right on the Download tab — Section 14.2) */}
      {activeFooterJobs.length > 0 && (
        <div
          style={{
            borderTop: "1px solid var(--line)",
            background: "var(--surface-1)",
            padding: "8px 24px",
          }}
        >
          <div
            style={{
              maxWidth: 940,
              margin: "0 auto",
              display: "flex",
              alignItems: "center",
              gap: 14,
            }}
          >
            <span
              className="font-display"
              style={{ fontSize: 11, color: "var(--text-2)", flexShrink: 0 }}
            >
              {t.download.inProgress}
            </span>
            <div style={{ flex: 1, display: "grid", gridTemplateColumns: `repeat(${activeFooterJobs.length}, 1fr)`, gap: 12 }}>
              {activeFooterJobs.map((j) => (
                <button
                  key={j.id}
                  type="button"
                  onClick={() => setActiveTab("queue")}
                  style={{
                    background: "var(--surface-2)",
                    border: "1px solid var(--line)",
                    borderRadius: 8,
                    padding: "6px 10px",
                    textAlign: "left",
                    cursor: "pointer",
                    display: "flex",
                    flexDirection: "column",
                    gap: 4,
                    minWidth: 0,
                  }}
                >
                  <div
                    style={{
                      display: "flex",
                      justifyContent: "space-between",
                      fontSize: 12,
                      gap: 8,
                    }}
                  >
                    <span
                      style={{
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                        color: "var(--text)",
                        fontWeight: 600,
                      }}
                    >
                      {j.title}
                    </span>
                    <span className="font-mono-num" style={{ color: "var(--text-2)", flexShrink: 0 }}>
                      {Math.round(j.progress.pct * 100)}%
                    </span>
                  </div>
                  <StripeProgress
                    pct={j.progress.pct}
                    state={j.state}
                    speedBps={j.progress.speed_bps}
                    height={6}
                  />
                </button>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
