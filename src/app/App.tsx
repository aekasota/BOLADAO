import React, { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ClipboardPaste, Maximize2, Minus, Square, X } from "lucide-react";
import {
  api,
  AppUpdateInfo,
  BinsStatusPayload,
  GameModeStatus,
  isTauriRuntime,
  Job,
  JobState,
  LaunchMode,
  LimiterStatsPayload,
  Progress,
  readInitialLaunchMode,
  safeListen,
} from "../lib/ipc";
import { formatSpeedPrimary } from "../lib/format";
import { OpArtCanvas } from "../shader/OpArtCanvas";
import { UpdateChip } from "./UpdateNotice";
import { LanguageChip } from "./LanguageChip";
import { ResourcesChip } from "./ResourcesChip";
import { HTML_LANG, loadSystemLanguage, useLang, useT } from "../i18n";
import { AppTab, useAppStore } from "../stores/appStore";
import { useLimiterStore } from "../stores/limiterStore";
import { useQueueStore } from "../stores/queueStore";
import { useSettingsStore } from "../stores/settingsStore";
import { DownloadTab } from "../features/download/DownloadTab";
import { QueueTab } from "../features/queue/QueueTab";
import { LibraryTab } from "../features/library/LibraryTab";
import { SettingsTab } from "../features/settings/SettingsTab";
import { InstallerView } from "../features/installer/InstallerView";
import { SPRING_UI, StripeProgress, Tooltip } from "../ui/Primitives";

export const App: React.FC = () => {
  const [launchMode] = useState<LaunchMode>(() => readInitialLaunchMode());
  const lang = useLang();
  useEffect(() => {
    loadSystemLanguage();
  }, []);
  useEffect(() => {
    document.documentElement.lang = HTML_LANG[lang];
  }, [lang]);
  if (launchMode.mode !== "app") {
    return (
      <Tooltip.Provider delayDuration={250}>
        <InstallerView launchMode={launchMode} />
      </Tooltip.Provider>
    );
  }
  return <MainApp />;
};

const MainApp: React.FC = () => {
  const t = useT();
  const urlInputRef = useRef<HTMLInputElement | null>(null);

  const activeTab = useAppStore((s) => s.activeTab);
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const setStageRect = useAppStore((s) => s.setStageRect);
  const setDragOverActive = useAppStore((s) => s.setDragOverActive);
  const fireRipple = useAppStore((s) => s.fireRipple);
  const clipboardSuggestion = useAppStore((s) => s.clipboardSuggestion);
  const setClipboardSuggestion = useAppStore((s) => s.setClipboardSuggestion);
  const setPendingUrlToLoad = useAppStore((s) => s.setPendingUrlToLoad);
  const binsStatus = useAppStore((s) => s.binsStatus);
  const setBinsStatus = useAppStore((s) => s.setBinsStatus);

  const settings = useSettingsStore((s) => s.settings);
  const loadSettings = useSettingsStore((s) => s.loadSettings);

  const jobs = useQueueStore((s) => s.jobs);
  const selectedIds = useQueueStore((s) => s.selectedIds);
  const loadQueue = useQueueStore((s) => s.loadQueue);
  const setJobs = useQueueStore((s) => s.setJobs);
  const applyProgressEvent = useQueueStore((s) => s.applyProgressEvent);

  const limiterStats = useLimiterStore((s) => s.stats);
  const setLimiterStats = useLimiterStore((s) => s.setStats);
  const setGameMode = useLimiterStore((s) => s.setGameMode);

  const [updateBannerVer, setUpdateBannerVer] = useState<string | null>(null);
  const [appUpdate, setAppUpdate] = useState<AppUpdateInfo | null>(null);
  const [applyingUpdate, setApplyingUpdate] = useState(false);
  const [updatingYtdlp, setUpdatingYtdlp] = useState(false);
  const [ytdlpUpdateError, setYtdlpUpdateError] = useState<string | null>(null);
  const [maximized, setMaximized] = useState(false);

  // Shrink stage to the top Notch crown strip whenever user switches away from Download tab (Section 13.5)
  useEffect(() => {
    if (activeTab !== "download") {
      setStageRect({ x: 0, y: 0, width: window.innerWidth, height: 52 });
    }
  }, [activeTab, setStageRect]);

  // Initial load + Tauri backend event subscriptions
  useEffect(() => {
    loadSettings();
    loadQueue();
    api.getBinsStatus().then(setBinsStatus);
    api.getGameModeStatus().then(setGameMode);
    api.getPendingUpdate().then((u) => u && setAppUpdate(u));

    let unlisteners: (() => void)[] = [];
    (async () => {
      unlisteners.push(
        await safeListen<BinsStatusPayload>("bins:status", (st) => {
          setBinsStatus(st);
          if (st.update_available) setUpdateBannerVer(st.update_available);
        })
      );
      unlisteners.push(
        await safeListen<string>("bins:update_available", (ver) => {
          setUpdateBannerVer(ver);
        })
      );
      unlisteners.push(
        await safeListen<Job[]>("queue:changed", (list) => {
          setJobs(list);
        })
      );
      unlisteners.push(
        await safeListen<{
          id: string;
          state: JobState;
          progress: Progress;
          equivalent_cmd: string;
        }>("job:progress", (ev) => {
          applyProgressEvent(ev);
        })
      );
      unlisteners.push(
        await safeListen<string>("job:completed", (jobId) => {
          // Ripple on background from the position of the completed item! (Section 13.6)
          const el = document.querySelector(`[data-job-row-id="${jobId}"]`);
          if (el) {
            const r = el.getBoundingClientRect();
            fireRipple(r.left + r.width * 0.5, r.top + r.height * 0.5, 1.15);
          } else {
            fireRipple(window.innerWidth * 0.5, window.innerHeight * 0.65, 1.0);
          }
        })
      );
      unlisteners.push(
        await safeListen<LimiterStatsPayload>("limiter:stats", (st) => {
          setLimiterStats(st);
        })
      );
      unlisteners.push(
        await safeListen<GameModeStatus>("gamemode:changed", (gm) => {
          setGameMode(gm);
        })
      );
      unlisteners.push(
        await safeListen<AppUpdateInfo>("app:update_ready", (u) => {
          setAppUpdate(u);
        })
      );
    })();

    return () => {
      unlisteners.forEach((fn) => fn());
    };
  }, []);

  // Window Focus Clipboard Detector -> discreet 6s pill (Section 6)
  useEffect(() => {
    let hideTimer: ReturnType<typeof setTimeout> | null = null;
    const seenClipboard = new Set<string>();

    const checkClipboardOnFocus = async () => {
      try {
        const text = await navigator.clipboard.readText();
        const trimmed = text?.trim();
        if (!trimmed || !trimmed.startsWith("http") || seenClipboard.has(trimmed)) {
          return;
        }
        const alreadyInQueue = useQueueStore
          .getState()
          .jobs.some((j) => j.url === trimmed);
        if (alreadyInQueue) return;

        const cls = await api.classifyUrl(trimmed);
        if (cls && !cls.is_drm && cls.platform_id !== "generic") {
          seenClipboard.add(trimmed);
          setClipboardSuggestion({ url: trimmed, platformName: cls.platform_name });
          if (hideTimer) clearTimeout(hideTimer);
          hideTimer = setTimeout(() => setClipboardSuggestion(null), 6000);
        }
      } catch {
        // Ignore clipboard read denials when unfocused
      }
    };

    window.addEventListener("focus", checkClipboardOnFocus);
    return () => {
      window.removeEventListener("focus", checkClipboardOnFocus);
      if (hideTimer) clearTimeout(hideTimer);
    };
  }, [setClipboardSuggestion]);

  // Global keyboard shortcuts (Section 14.7) + Global Paste & Drag-and-Drop
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const inInput =
        target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA");

      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "l") {
        e.preventDefault();
        setActiveTab("download");
        setTimeout(() => urlInputRef.current?.focus(), 20);
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key === ",") {
        e.preventDefault();
        setActiveTab("settings");
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "p") {
        e.preventDefault();
        api.pauseAll();
        return;
      }
      if ((e.ctrlKey || e.metaKey) && ["1", "2", "3", "4"].includes(e.key)) {
        e.preventDefault();
        const map: Record<string, AppTab> = {
          "1": "download",
          "2": "queue",
          "3": "library",
          "4": "settings",
        };
        setActiveTab(map[e.key]);
        return;
      }

      if (!inInput && activeTab === "queue" && selectedIds.length > 0) {
        if (e.key === " ") {
          e.preventDefault();
          const first = jobs.find((j) => j.id === selectedIds[0]);
          if (first) {
            if (first.state === "paused") api.resume(first.id);
            else api.pause(first.id);
          }
        } else if (e.key === "Delete") {
          e.preventDefault();
          api.removeJobs(selectedIds);
        }
      }
    };

    const onPaste = (e: ClipboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) {
        return;
      }
      const pasted = e.clipboardData?.getData("text");
      if (pasted && pasted.trim()) {
        e.preventDefault();
        setActiveTab("download");
        setPendingUrlToLoad(pasted.trim());
      }
    };

    const onDragOver = (e: DragEvent) => {
      if (e.dataTransfer?.types.includes("text/uri-list") || e.dataTransfer?.types.includes("text/plain")) {
        e.preventDefault();
        setDragOverActive(true);
      }
    };
    const onDragLeave = (e: DragEvent) => {
      if (e.relatedTarget === null) {
        setDragOverActive(false);
      }
    };
    const onDrop = (e: DragEvent) => {
      setDragOverActive(false);
      const text =
        e.dataTransfer?.getData("text/uri-list") || e.dataTransfer?.getData("text/plain");
      if (text && text.trim()) {
        e.preventDefault();
        setActiveTab("download");
        setPendingUrlToLoad(text.trim());
      }
    };

    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("paste", onPaste);
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("dragleave", onDragLeave);
    window.addEventListener("drop", onDrop);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("paste", onPaste);
      window.removeEventListener("dragover", onDragOver);
      window.removeEventListener("dragleave", onDragLeave);
      window.removeEventListener("drop", onDrop);
    };
  }, [activeTab, selectedIds, jobs]);

  const queueCounts = useMemo(() => {
    let active = 0;
    let queued = 0;
    for (const j of jobs) {
      if (
        ["downloading", "probing", "postprocessing", "workshop", "waiting_premiere"].includes(
          j.state
        )
      ) {
        active += 1;
      } else if (j.state === "queued" || j.state === "paused") {
        queued += 1;
      }
    }
    return { active, queued, badge: active + queued };
  }, [jobs]);

  const sparklinePath = useMemo(() => {
    const pts = limiterStats.sparkline_60s;
    if (!pts || pts.length === 0) return "";
    const max = Math.max(...pts, 64 * 1024);
    const w = 68;
    const h = 16;
    return pts
      .map((v, i) => {
        const x = (i / Math.max(1, pts.length - 1)) * w;
        const y = h - (v / max) * (h - 2) - 1;
        return `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" ");
  }, [limiterStats.sparkline_60s]);

  const winMinimize = () => isTauriRuntime() && getCurrentWindow().minimize();
  const winToggleMax = async () => {
    if (!isTauriRuntime()) return;
    const w = getCurrentWindow();
    await w.toggleMaximize();
    setMaximized(await w.isMaximized());
  };
  const winClose = () => isTauriRuntime() && getCurrentWindow().close();

  const applyAppUpdate = async () => {
    setApplyingUpdate(true);
    try {
      await api.applyUpdateNow();
    } catch {
      setApplyingUpdate(false);
    }
  };

  const applyYtdlpUpdate = async () => {
    setUpdatingYtdlp(true);
    setYtdlpUpdateError(null);
    try {
      await api.updateYtdlp();
      setUpdateBannerVer(null);
      setBinsStatus(await api.getBinsStatus());
    } catch (e) {
      setYtdlpUpdateError(String(e));
    } finally {
      setUpdatingYtdlp(false);
    }
  };

  return (
    <Tooltip.Provider delayDuration={250}>
      <div
        onClick={(e) => {
          // Click on empty background triggers a subtle ripple (Section 13.6)
          if (e.target === e.currentTarget) {
            fireRipple(e.clientX, e.clientY, 0.75);
          }
        }}
        style={{
          width: "100vw",
          height: "100vh",
          display: "flex",
          flexDirection: "column",
          position: "relative",
          overflow: "hidden",
        }}
      >
        {/* Fullscreen WebGL2 Op-Art Background */}
        <OpArtCanvas />

        {/* Sculpted Top-Docked Architectural Notch + Corner Docks */}
        <header
          className="notch-header-shell"
          data-tauri-drag-region
          onDoubleClick={(e) => {
            if (e.target === e.currentTarget) winToggleMax();
          }}
        >
          <div className="notch-top-rim" />

          {/* Left Brand Corner Pill with Live Op-Art Cutout Hole ("furo") */}
          <div
            className="brand-corner-pill"
            data-tauri-drag-region
          >
            <div className="brand-pill-bg-mask" />
            <div className="brand-hole-badge" title="Boladão" />
            <span
              className="font-display"
              data-tauri-drag-region
              style={{ fontSize: 13.5, letterSpacing: "0.03em", pointerEvents: "none" }}
            >
              BOLADÃO
            </span>
          </div>

          {/* Center Top-Docked Notch (G1-continuous concave shoulder fillets + sliding Op-Art pill) */}
          <motion.nav
            layout
            transition={settings.reduce_motion ? { duration: 0 } : SPRING_UI}
            aria-label={t.app.mainNav}
            className="notch-dock"
          >
            {/* Left Concave Shoulder Fillet */}
            <svg
              className="notch-ear notch-ear-left"
              viewBox="0 0 14 14"
              fill="none"
              aria-hidden="true"
            >
              <path d="M 0 0 L 14 0 L 14 14 A 14 14 0 0 0 0 0 Z" fill="var(--surface-1)" />
              <path
                d="M 0 0.5 A 13.5 13.5 0 0 1 13.5 14"
                stroke="var(--line)"
                strokeWidth="1"
              />
            </svg>

            {(
              [
                { id: "download", label: t.app.tabs.download },
                { id: "queue", label: t.app.tabs.queue, badge: queueCounts.badge },
                { id: "library", label: t.app.tabs.library },
                { id: "settings", label: t.app.tabs.settings },
              ] as { id: AppTab; label: string; badge?: number }[]
            ).map((t) => {
              const selected = activeTab === t.id;
              return (
                <button
                  key={t.id}
                  type="button"
                  data-active={selected ? "true" : "false"}
                  onClick={() => setActiveTab(t.id)}
                  className="notch-tab-btn font-display"
                  style={{
                    color: selected ? "var(--on-accent)" : "var(--text-2)",
                  }}
                >
                  {selected && (
                    <motion.span
                      layoutId="notch-active-pill"
                      transition={settings.reduce_motion ? { duration: 0 } : SPRING_UI}
                      style={{
                        position: "absolute",
                        inset: 0,
                        borderRadius: 8,
                        background: "var(--accent)",
                        zIndex: -1,
                      }}
                    />
                  )}
                  <span>{t.label}</span>
                  {typeof t.badge === "number" && t.badge > 0 && (
                    <motion.span
                      key={t.badge}
                      initial={{ scale: 0.7 }}
                      animate={{ scale: 1 }}
                      transition={SPRING_UI}
                      className="font-mono-num"
                      style={{
                        padding: "0 6px",
                        height: 17,
                        borderRadius: 999,
                        background: selected ? "var(--on-accent)" : "var(--accent)",
                        color: selected ? "var(--accent)" : "var(--on-accent)",
                        fontSize: 10.5,
                        fontWeight: 750,
                        display: "inline-flex",
                        alignItems: "center",
                      }}
                    >
                      {t.badge}
                    </motion.span>
                  )}
                </button>
              );
            })}

            {/* Right Concave Shoulder Fillet */}
            <svg
              className="notch-ear notch-ear-right"
              viewBox="0 0 14 14"
              fill="none"
              aria-hidden="true"
            >
              <path d="M 0 0 L 14 0 L 0 14 Z" fill="none" />
              <path d="M 0 0 L 14 0 A 14 14 0 0 0 0 14 Z" fill="var(--surface-1)" />
              <path
                d="M 14 0.5 A 13.5 13.5 0 0 0 0.5 14"
                stroke="var(--line)"
                strokeWidth="1"
              />
            </svg>
          </motion.nav>

          {/* Right Window Controls Pill */}
          <div className="window-controls-pill">
            <button
              type="button"
              onClick={winMinimize}
              aria-label={t.app.window.minimize}
              className="win-ctrl-btn"
            >
              <Minus size={13} />
            </button>
            <button
              type="button"
              onClick={winToggleMax}
              aria-label={t.app.window.maximize}
              className="win-ctrl-btn"
            >
              {maximized ? <Square size={11} /> : <Maximize2 size={12} />}
            </button>
            <button
              type="button"
              onClick={winClose}
              aria-label={t.app.window.close}
              className="win-ctrl-btn win-ctrl-close"
            >
              <X size={14} />
            </button>
          </div>
        </header>


        {/* Discreet 6-second Clipboard Pill when window regains focus (Section 6) */}
        <AnimatePresence>
          {clipboardSuggestion && (
            <motion.div
              initial={{ opacity: 0, y: -12 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -10 }}
              transition={SPRING_UI}
              style={{
                position: "absolute",
                top: 54,
                right: 24,
                zIndex: 40,
              }}
            >
              <div
                className="op-panel"
                style={{
                  padding: "6px 10px 6px 12px",
                  borderRadius: 999,
                  display: "inline-flex",
                  alignItems: "center",
                  gap: 8,
                  borderColor: "var(--accent)",
                }}
              >
                <ClipboardPaste size={14} color="var(--accent-text)" />
                <button
                  type="button"
                  onClick={() => {
                    setActiveTab("download");
                    setPendingUrlToLoad(clipboardSuggestion.url);
                    setClipboardSuggestion(null);
                  }}
                  style={{
                    background: "transparent",
                    border: "none",
                    color: "var(--text)",
                    fontSize: 12.5,
                    fontWeight: 650,
                    cursor: "pointer",
                  }}
                >
                  {t.app.clipboardPaste(clipboardSuggestion.platformName)}
                </button>
                <button
                  type="button"
                  onClick={() => setClipboardSuggestion(null)}
                  style={{
                    background: "transparent",
                    border: "none",
                    color: "var(--text-3)",
                    cursor: "pointer",
                    display: "flex",
                  }}
                >
                  <X size={13} />
                </button>
              </div>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Main Viewport OR First-Run Extraction Screen ("Preparando os motores" — Section 3.2) */}
        <main style={{ flex: 1, minHeight: 0, position: "relative" }}>
          {binsStatus.extracting ? (
            <div
              style={{
                height: "100%",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                padding: 24,
                position: "relative",
                zIndex: 10,
              }}
            >
              <div
                className="op-panel"
                style={{
                  width: "100%",
                  maxWidth: 460,
                  padding: 28,
                  display: "flex",
                  flexDirection: "column",
                  gap: 14,
                }}
              >
                <div className="font-display" style={{ fontSize: 22 }}>
                  {t.app.enginesTitle}
                </div>
                <div style={{ fontSize: 13, color: "var(--text-2)" }}>{t.app.enginesBody}</div>
                <StripeProgress
                  pct={binsStatus.progress_pct}
                  state="downloading"
                  speedBps={8 * 1024 * 1024}
                  height={12}
                />
                <div
                  className="font-mono-num"
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    fontSize: 12,
                    color: "var(--text-2)",
                  }}
                >
                  <span>{binsStatus.stage}</span>
                  <span>{Math.round(binsStatus.progress_pct * 100)}%</span>
                </div>
              </div>
            </div>
          ) : (
            <>
              {activeTab === "download" && <DownloadTab urlInputRef={urlInputRef} />}
              {activeTab === "queue" && <QueueTab />}
              {activeTab === "library" && <LibraryTab />}
              {activeTab === "settings" && <SettingsTab />}
            </>
          )}
        </main>

        {/* Bottom 32px Status Bar — Always Visible (Section 14.1) */}
        <footer
          style={{
            height: 32,
            background: "var(--surface-1)",
            borderTop: "1px solid var(--line)",
            padding: "0 14px",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            fontSize: 12,
            zIndex: 20,
          }}
        >
          {/* Left: Total Speed + 60s Sparkline */}
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <span className="font-mono-num" style={{ fontWeight: 700, minWidth: 74 }}>
              {formatSpeedPrimary(limiterStats.speed_bps, settings.speed_unit)}
            </span>
            <svg width="68" height="16" aria-hidden="true">
              <path
                d={sparklinePath}
                fill="none"
                stroke="var(--accent)"
                strokeWidth="1.6"
                strokeLinejoin="round"
              />
            </svg>
          </div>

          {/* Center: Active & Queued Summary */}
          <div className="font-mono-num" style={{ color: "var(--text-2)" }}>
            {t.app.activeCount(queueCounts.active)} · {t.app.queuedCount(queueCounts.queued)}
          </div>

          {/* Right: updates ready, then language and bandwidth + disk, each with its own popover */}
          <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
            {/* Updates ready: Boladão (downloaded and verified) and the yt-dlp engine */}
            {appUpdate && (
              <UpdateChip
                label={t.update.appChip(appUpdate.version)}
                ariaLabel={t.update.appAria(appUpdate.version)}
                title={t.update.appTitle(appUpdate.version)}
                body={t.update.appBody}
                actionLabel={t.update.restartNow}
                busyLabel={t.update.restarting}
                laterLabel={t.common.later}
                busy={applyingUpdate}
                autoOpen
                onAction={applyAppUpdate}
              />
            )}
            {updateBannerVer && (
              <UpdateChip
                label={`yt-dlp ${updateBannerVer}`}
                ariaLabel={t.update.ytdlpAria(updateBannerVer)}
                title={t.update.ytdlpTitle}
                body={t.update.ytdlpBody(updateBannerVer)}
                actionLabel={t.update.updateNow}
                busyLabel={t.update.updating}
                laterLabel={t.common.later}
                busy={updatingYtdlp}
                error={ytdlpUpdateError}
                autoOpen={!appUpdate}
                onAction={applyYtdlpUpdate}
              />
            )}
            <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <LanguageChip />
              <ResourcesChip />
            </div>

            <span
              aria-hidden="true"
              style={{
                width: 1,
                height: 12,
                background: "var(--line)",
                display: "inline-block",
              }}
            />
            <span
              role="link"
              tabIndex={0}
              aria-label={t.app.authorLink}
              onClick={() => api.openAuthorProfile()}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  api.openAuthorProfile();
                }
              }}
              className="font-mono-num"
              style={{
                fontSize: 11,
                letterSpacing: "0.02em",
                color: "var(--text-3)",
                display: "inline-flex",
                alignItems: "center",
                gap: 1,
                userSelect: "none",
                cursor: "pointer",
              }}
            >
              <span style={{ color: "var(--accent-text)", fontWeight: 700 }}>@</span>
              <span>aekasota</span>
            </span>
          </div>
        </footer>
      </div>
    </Tooltip.Provider>
  );
};

const winBtnStyle: React.CSSProperties = {
  width: 34,
  height: 30,
  borderRadius: 6,
  border: "none",
  background: "transparent",
  color: "var(--text-2)",
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  cursor: "pointer",
};
