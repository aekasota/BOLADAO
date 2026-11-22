import React, { useEffect, useState } from "react";
import { motion } from "motion/react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Check, Download, Minus, Play, Trash2, X } from "lucide-react";
import {
  api,
  BinsStatusPayload,
  InstallerDefaults,
  InstallerProgressPayload,
  isTauriRuntime,
  LaunchMode,
  safeListen,
} from "../../lib/ipc";
import { OpArtCanvas } from "../../shader/OpArtCanvas";
import { useAppStore } from "../../stores/appStore";
import {
  Button,
  PathPreview,
  SPRING_UI,
  StripeProgress,
  Toggle,
} from "../../ui/Primitives";
import { useT } from "../../i18n";

interface Props {
  launchMode: Extract<LaunchMode, { mode: "installer" } | { mode: "uninstaller" }>;
}

export const InstallerView: React.FC<Props> = ({ launchMode }) => {
  const t = useT();
  const fireRipple = useAppStore((s) => s.fireRipple);

  const isUninstaller = launchMode.mode === "uninstaller";
  const [defaults, setDefaults] = useState<InstallerDefaults | null>(null);
  const [installDir, setInstallDir] = useState("C:\\Users\\Public\\Boladão");
  const [desktopShortcut, setDesktopShortcut] = useState(true);
  const [startMenuShortcut, setStartMenuShortcut] = useState(true);

  const [phase, setPhase] = useState<"ready" | "running" | "done" | "error">("ready");
  const [pct, setPct] = useState(0);
  const [stage, setStage] = useState(() => t.installer.starting);
  const [installedExe, setInstalledExe] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  // Uninstaller state
  const [removeUserData, setRemoveUserData] = useState(false);

  useEffect(() => {
    if (!isUninstaller) {
      api.getInstallerDefaults().then((info) => {
        setDefaults(info);
        setInstallDir(info.default_install_dir);
      });
    }

    let unlisteners: (() => void)[] = [];
    (async () => {
      unlisteners.push(
        await safeListen<InstallerProgressPayload>("installer:progress", (ev) => {
          setPct(ev.pct);
          setStage(ev.stage);
          if (ev.error) {
            setErrorMsg(ev.error);
            setPhase("error");
          } else if (ev.done) {
            if (ev.installed_exe) setInstalledExe(ev.installed_exe);
            setPhase("done");
            fireRipple(window.innerWidth * 0.5, window.innerHeight * 0.5, 1.25);
          }
        })
      );
      unlisteners.push(
        await safeListen<BinsStatusPayload>("bins:status", (st) => {
          if (st.extracting) {
            // Map binary extraction progress smoothly inside the 0.46 -> 0.84 window
            setPct((cur) => Math.max(cur, 0.46 + st.progress_pct * 0.38));
            setStage(st.stage);
          }
        })
      );
    })();

    return () => {
      unlisteners.forEach((fn) => fn());
    };
  }, [isUninstaller, fireRipple]);

  const pickInstallFolder = async () => {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      defaultPath: installDir,
      title: t.installer.pickFolderTitle,
    });
    if (typeof selected === "string" && selected.trim()) {
      const clean = selected.replace(/[\\/]+$/, "");
      const finalDir = clean.toLowerCase().endsWith("boladão") || clean.toLowerCase().endsWith("boladao")
        ? clean
        : `${clean}\\Boladão`;
      setInstallDir(finalDir);
    }
  };

  const handleInstall = async () => {
    setErrorMsg(null);
    setPhase("running");
    setPct(0.04);
    setStage(t.installer.preparing);
    try {
      const exePath = await api.installBoladao(
        installDir,
        desktopShortcut,
        startMenuShortcut
      );
      setInstalledExe(exePath);
      setPct(1);
      setPhase("done");
      fireRipple(window.innerWidth * 0.5, window.innerHeight * 0.5, 1.25);
    } catch (e: any) {
      setErrorMsg(String(e));
      setPhase("error");
    }
  };

  const handleUninstall = async () => {
    if (launchMode.mode !== "uninstaller") return;
    setErrorMsg(null);
    setPhase("running");
    setPct(0.45);
    setStage(t.installer.removing);
    try {
      await api.uninstallBoladao(launchMode.target_dir, removeUserData);
      setPct(1);
      setPhase("done");
      fireRipple(window.innerWidth * 0.5, window.innerHeight * 0.5, 1.1);
    } catch (e: any) {
      setErrorMsg(String(e));
      setPhase("error");
    }
  };

  const winMinimize = () => isTauriRuntime() && getCurrentWindow().minimize();
  const winClose = () => {
    if (!isTauriRuntime()) return;
    if (isUninstaller) {
      api.closeUninstaller();
    } else {
      getCurrentWindow().close();
    }
  };

  return (
    <div
      onClick={(e) => {
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
      {/* Live Fullscreen WebGL2 Op-Art Background */}
      <OpArtCanvas />

      {/* Top Architectural Notch Header matching the main app */}
      <header className="notch-header-shell" data-tauri-drag-region>
        <div className="notch-top-rim" />

        <div className="brand-corner-pill" data-tauri-drag-region>
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

        <nav aria-label={t.installer.modeNav} className="notch-dock">
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

          <div
            className="notch-tab-btn font-display"
            style={{
              background: "var(--accent)",
              color: "var(--on-accent)",
              cursor: "default",
              padding: "0 16px",
              fontSize: 11.5,
              letterSpacing: "0.03em",
            }}
          >
            {isUninstaller ? t.installer.uninstallTab : t.installer.installerTab}
          </div>

          <svg
            className="notch-ear notch-ear-right"
            viewBox="0 0 14 14"
            fill="none"
            aria-hidden="true"
          >
            <path d="M 0 0 L 14 0 A 14 14 0 0 0 0 14 Z" fill="var(--surface-1)" />
            <path
              d="M 14 0.5 A 13.5 13.5 0 0 0 0.5 14"
              stroke="var(--line)"
              strokeWidth="1"
            />
          </svg>
        </nav>

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
            onClick={winClose}
            aria-label={t.app.window.close}
            disabled={phase === "running"}
            className="win-ctrl-btn win-ctrl-close"
            style={{ opacity: phase === "running" ? 0.4 : 1 }}
          >
            <X size={14} />
          </button>
        </div>
      </header>

      {/* Center Stage Card */}
      <main
        style={{
          flex: 1,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          padding: "12px 24px",
          position: "relative",
          zIndex: 10,
        }}
      >
        <motion.div
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
          transition={SPRING_UI}
          className="op-panel"
          style={{
            width: "100%",
            maxWidth: isUninstaller ? 540 : 620,
            padding: "22px 26px",
            display: "flex",
            flexDirection: "column",
            gap: 16,
          }}
        >
          {!isUninstaller ? (
            <>
              {phase === "ready" && (
                <>
                  <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                    <h1 className="font-display" style={{ margin: 0, fontSize: 21 }}>
                      {defaults?.already_installed
                        ? t.installer.titleUpdate
                        : t.installer.titleInstall}
                    </h1>
                    <div style={{ fontSize: 13, color: "var(--text-2)" }}>
                      {t.installer.bundleBefore}{" "}
                      <strong style={{ color: "var(--text)" }}>yt-dlp</strong>,{" "}
                      <strong style={{ color: "var(--text)" }}>Deno</strong>,{" "}
                      <strong style={{ color: "var(--text)" }}>FFmpeg 9.0.2</strong> {t.installer.and}{" "}
                      {t.installer.bundleAfter}
                    </div>
                  </div>

                  <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                    <span
                      className="font-display"
                      style={{ fontSize: 11.5, color: "var(--text-2)", letterSpacing: "0.02em" }}
                    >
                      {t.installer.installFolder}
                    </span>
                    <PathPreview path={installDir} onClick={pickInstallFolder} />
                    <span className="font-mono-num" style={{ fontSize: 11, color: "var(--text-3)" }}>
                      {t.installer.mediaFolders}
                    </span>
                  </div>

                  <div
                    style={{
                      display: "grid",
                      gridTemplateColumns: "1fr 1fr",
                      gap: 10,
                    }}
                  >
                    <div
                      style={{
                        padding: "10px 12px",
                        borderRadius: 8,
                        background: "var(--surface-2)",
                        border: "1px solid var(--line)",
                      }}
                    >
                      <Toggle
                        checked={desktopShortcut}
                        onChange={setDesktopShortcut}
                        label={t.installer.desktopShortcut}
                      />
                    </div>
                    <div
                      style={{
                        padding: "10px 12px",
                        borderRadius: 8,
                        background: "var(--surface-2)",
                        border: "1px solid var(--line)",
                      }}
                    >
                      <Toggle
                        checked={startMenuShortcut}
                        onChange={setStartMenuShortcut}
                        label={t.installer.startMenuShortcut}
                      />
                    </div>
                  </div>

                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      gap: 12,
                      paddingTop: 4,
                    }}
                  >
                    <div
                      className="font-mono-num"
                      style={{
                        display: "flex",
                        gap: 6,
                        flexWrap: "wrap",
                        fontSize: 11,
                        color: "var(--text-2)",
                      }}
                    >
                      <span
                        style={{
                          padding: "3px 8px",
                          borderRadius: 6,
                          background: "var(--surface-2)",
                          border: "1px solid var(--line)",
                        }}
                      >
                        yt-dlp 2026.10.26
                      </span>
                      <span
                        style={{
                          padding: "3px 8px",
                          borderRadius: 6,
                          background: "var(--surface-2)",
                          border: "1px solid var(--line)",
                        }}
                      >
                        FFmpeg 9.0.2
                      </span>
                      <span
                        style={{
                          padding: "3px 8px",
                          borderRadius: 6,
                          background: "var(--surface-2)",
                          border: "1px solid var(--line)",
                        }}
                      >
                        Deno 2.9.8
                      </span>
                    </div>

                    <Button
                      variant="primary"
                      size="lg"
                      onClick={handleInstall}
                      className="font-display"
                      style={{ fontSize: 13, letterSpacing: "0.02em", padding: "0 22px" }}
                    >
                      <Download size={16} />
                      {defaults?.already_installed ? t.installer.updateNow : t.installer.installNow}
                    </Button>
                  </div>
                </>
              )}

              {phase === "running" && (
                <div style={{ display: "flex", flexDirection: "column", gap: 14, padding: "8px 0" }}>
                  <div className="font-display" style={{ fontSize: 20 }}>
                    {t.installer.installing}
                  </div>
                  <div style={{ fontSize: 13, color: "var(--text-2)" }}>
                    {t.installer.installingBody}
                  </div>
                  <StripeProgress
                    pct={pct}
                    state="downloading"
                    speedBps={9 * 1024 * 1024}
                    height={14}
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
                    <span>{stage}</span>
                    <span>{Math.round(pct * 100)}%</span>
                  </div>
                </div>
              )}

              {phase === "done" && (
                <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                    <div
                      style={{
                        width: 36,
                        height: 36,
                        borderRadius: 8,
                        background: "var(--accent)",
                        color: "var(--on-accent)",
                        display: "flex",
                        alignItems: "center",
                        justifyContent: "center",
                        flexShrink: 0,
                      }}
                    >
                      <Check size={20} strokeWidth={2.8} />
                    </div>
                    <div>
                      <h2 className="font-display" style={{ margin: 0, fontSize: 20 }}>
                        {t.installer.doneTitle}
                      </h2>
                      <div style={{ fontSize: 13, color: "var(--text-2)" }}>
                        {t.installer.doneBody}
                      </div>
                    </div>
                  </div>

                  <div
                    className="font-mono-num"
                    style={{
                      padding: "10px 12px",
                      borderRadius: 8,
                      background: "var(--surface-2)",
                      border: "1px solid var(--line)",
                      fontSize: 11.5,
                      color: "var(--text-2)",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {t.installer.installedAt(installedExe || `${installDir}\\Boladão.exe`)}
                  </div>

                  <div style={{ display: "flex", justifyContent: "flex-end", gap: 10 }}>
                    <Button variant="secondary" onClick={winClose}>
                      {t.installer.finish}
                    </Button>
                    <Button
                      variant="primary"
                      size="lg"
                      className="font-display"
                      style={{ fontSize: 13, letterSpacing: "0.02em" }}
                      onClick={() => {
                        const exe = installedExe || `${installDir}\\Boladão.exe`;
                        api.launchInstalledBoladao(exe);
                      }}
                    >
                      <Play size={15} /> {t.installer.openApp}
                    </Button>
                  </div>
                </div>
              )}

              {phase === "error" && (
                <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
                  <div className="font-display" style={{ fontSize: 19, color: "var(--warn)" }}>
                    {t.installer.failedTitle}
                  </div>
                  <div
                    className="font-mono-num"
                    style={{
                      padding: 12,
                      borderRadius: 8,
                      background: "var(--surface-2)",
                      border: "1px solid var(--warn)",
                      fontSize: 12,
                      color: "var(--text)",
                    }}
                  >
                    {errorMsg}
                  </div>
                  <div style={{ display: "flex", justifyContent: "flex-end", gap: 10 }}>
                    <Button variant="secondary" onClick={() => setPhase("ready")}>
                      {t.installer.back}
                    </Button>
                    <Button variant="primary" onClick={handleInstall}>
                      {t.installer.tryAgain}
                    </Button>
                  </div>
                </div>
              )}
            </>
          ) : (
            /* UNINSTALLER MODE */
            <>
              {phase === "ready" && (
                <>
                  <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                    <h1 className="font-display" style={{ margin: 0, fontSize: 20 }}>
                      {t.installer.uninstallTitle}
                    </h1>
                    <div style={{ fontSize: 13, color: "var(--text-2)" }}>
                      {t.installer.keepBefore}{" "}
                      <strong style={{ color: "var(--text)" }}>{t.installer.musicFolder}</strong>{" "}
                      {t.installer.and}{" "}
                      <strong style={{ color: "var(--text)" }}>{t.installer.videosFolder}</strong>{" "}
                      {t.installer.keepAfter}
                    </div>
                  </div>

                  <div
                    style={{
                      padding: "11px 14px",
                      borderRadius: 8,
                      background: "var(--surface-2)",
                      border: "1px solid var(--line)",
                    }}
                  >
                    <Toggle
                      checked={removeUserData}
                      onChange={setRemoveUserData}
                      label={t.installer.removeData}
                    />
                  </div>

                  <div style={{ display: "flex", justifyContent: "flex-end", gap: 10 }}>
                    <Button variant="secondary" onClick={winClose}>
                      {t.installer.cancel}
                    </Button>
                    <Button
                      variant="primary"
                      onClick={handleUninstall}
                      className="font-display"
                      style={{ fontSize: 12.5, letterSpacing: "0.02em" }}
                    >
                      <Trash2 size={15} /> {t.installer.uninstallButton}
                    </Button>
                  </div>
                </>
              )}

              {phase === "running" && (
                <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
                  <div className="font-display" style={{ fontSize: 19 }}>
                    {t.installer.removingTitle}
                  </div>
                  <StripeProgress
                    pct={pct}
                    state="downloading"
                    speedBps={8 * 1024 * 1024}
                    height={12}
                  />
                  <div className="font-mono-num" style={{ fontSize: 12, color: "var(--text-2)" }}>
                    {stage}
                  </div>
                </div>
              )}

              {phase === "done" && (
                <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
                  <div className="font-display" style={{ fontSize: 20 }}>
                    {t.installer.uninstalledTitle}
                  </div>
                  <div style={{ fontSize: 13, color: "var(--text-2)" }}>
                    {t.installer.uninstalledBody}
                  </div>
                  <div style={{ display: "flex", justifyContent: "flex-end" }}>
                    <Button
                      variant="primary"
                      onClick={winClose}
                      className="font-display"
                      style={{ fontSize: 12.5 }}
                    >
                      {t.installer.closeButton}
                    </Button>
                  </div>
                </div>
              )}

              {phase === "error" && (
                <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
                  <div className="font-display" style={{ fontSize: 18, color: "var(--warn)" }}>
                    {t.installer.uninstallError}
                  </div>
                  <div className="font-mono-num" style={{ fontSize: 12 }}>
                    {errorMsg}
                  </div>
                  <div style={{ display: "flex", justifyContent: "flex-end" }}>
                    <Button variant="secondary" onClick={winClose}>
                      {t.installer.close}
                    </Button>
                  </div>
                </div>
              )}
            </>
          )}
        </motion.div>
      </main>

      {/* Bottom 30px Architectural Bar with version + subtle @aekasota signature */}
      <footer
        style={{
          height: 30,
          background: "var(--surface-1)",
          borderTop: "1px solid var(--line)",
          padding: "0 14px",
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          fontSize: 11.5,
          zIndex: 20,
        }}
      >
        <span className="font-mono-num" style={{ color: "var(--text-3)" }}>
          Boladão v{defaults?.app_version || "1.0.0"} · Windows x64
        </span>
        <span
          className="font-mono-num"
          style={{
            fontSize: 11,
            letterSpacing: "0.02em",
            color: "var(--text-3)",
            display: "inline-flex",
            alignItems: "center",
            gap: 1,
          }}
        >
          <span style={{ color: "var(--accent-text)", fontWeight: 700 }}>@</span>
          <span>aekasota</span>
        </span>
      </footer>
    </div>
  );
};
