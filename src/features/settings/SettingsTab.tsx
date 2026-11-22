import React, { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  Cpu,
  FolderOpen,
  Gamepad2,
  Gauge,
  KeyRound,
  Palette,
  RefreshCw,
  RotateCcw,
  Sliders,
  Terminal,
  Trash2,
} from "lucide-react";
import {
  api,
  BandwidthProfile,
  CollisionPolicy,
  GameModeOverride,
  safeListen,
  SpeedUnit,
  ThemeId,
} from "../../lib/ipc";
import {
  bpsToLogSlider,
  formatDualBandwidth,
  logSliderToBps,
} from "../../lib/format";
import { useAppStore } from "../../stores/appStore";
import { useLimiterStore } from "../../stores/limiterStore";
import { useSettingsStore } from "../../stores/settingsStore";
import {
  Button,
  Chip,
  PathPreview,
  PlatformIcon,
  Segmented,
  Slider,
  Toggle,
} from "../../ui/Primitives";

const TEMPLATE_TOKENS = [
  { token: "{tipo}", label: "Tipo" },
  { token: "{plataforma}", label: "Plataforma" },
  { token: "{canal}", label: "Canal" },
  { token: "{artista}", label: "Artista" },
  { token: "{playlist}", label: "Playlist" },
  { token: "{album}", label: "Álbum" },
  { token: "{nn}", label: "NN" },
  { token: "{titulo}", label: "Título" },
  { token: "{faixa}", label: "Faixa" },
  { token: "{id}", label: "ID" },
  { token: "{ano}", label: "Ano" },
  { token: "{mes}", label: "Mês" },
];

const ACCOUNT_PLATFORMS = [
  { id: "youtube", name: "YouTube / YouTube Music", slug: "youtube", color: "#FF0033" },
  { id: "instagram", name: "Instagram", slug: "instagram", color: "#E4405F" },
  { id: "tiktok", name: "TikTok", slug: "tiktok", color: "#00F2EA" },
  { id: "twitter", name: "X / Twitter", slug: "x", color: "#F2EEE6" },
  { id: "facebook", name: "Facebook", slug: "facebook", color: "#0866FF" },
  { id: "twitch", name: "Twitch", slug: "twitch", color: "#9146FF" },
  { id: "vimeo", name: "Vimeo", slug: "vimeo", color: "#1AB7EA" },
];

export const SettingsTab: React.FC = () => {
  const settings = useSettingsStore((s) => s.settings);
  const presets = useSettingsStore((s) => s.presets);
  const updateSettings = useSettingsStore((s) => s.updateSettings);
  const loadSettings = useSettingsStore((s) => s.loadSettings);
  const refreshPresets = useSettingsStore((s) => s.refreshPresets);

  const activeSection = useAppStore((s) => s.settingsSection);
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const binsStatus = useAppStore((s) => s.binsStatus);
  const setBinsStatus = useAppStore((s) => s.setBinsStatus);

  const limiterStats = useLimiterStore((s) => s.stats);
  const gameMode = useLimiterStore((s) => s.gameMode);
  const setGameMode = useLimiterStore((s) => s.setGameMode);

  const [audioPreview, setAudioPreview] = useState("");
  const [videoPreview, setVideoPreview] = useState("");
  const [runningProcs, setRunningProcs] = useState<string[]>([]);
  const [newGameExe, setNewGameExe] = useState("");
  const [openLoginPlatform, setOpenLoginPlatform] = useState<string | null>(null);
  const [accountMsg, setAccountMsg] = useState<string | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    safeListen<string>("accounts:login_window_closed", (closedId) => {
      setOpenLoginPlatform((cur) => (cur === closedId ? null : cur));
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  // Components & supported sites state
  const [diagOutput, setDiagOutput] = useState<string | null>(null);
  const [updatingYtdlp, setUpdatingYtdlp] = useState(false);
  const [extractors, setExtractors] = useState<string[]>([]);
  const [extractorSearch, setExtractorSearch] = useState("");
  const [extraArgsText, setExtraArgsText] = useState(
    settings.global_extra_ytdlp_args.join(" ")
  );

  // Live path previews for Audio and Video templates
  useEffect(() => {
    api
      .previewPath(settings.music_root, settings.audio_template, {
        media_type: "Música",
        platform: "YouTube Music",
        creator: "Jorge Ben Jor",
        playlist: "A Tábua de Esmeralda",
        index: 1,
        title: "O Homem da Gravata Florida",
        track: "O Homem da Gravata Florida",
        id: "yt987654",
        year: "2026",
        month: "11",
        ext: "mp3",
      })
      .then(setAudioPreview);

    api
      .previewPath(settings.video_root, settings.video_template, {
        media_type: "Vídeos",
        platform: "YouTube",
        creator: "Canal Exemplo",
        playlist: "Favoritos",
        index: 3,
        title: "Documentário Completo",
        track: null,
        id: "dQw4w9WgXcQ",
        year: "2026",
        month: "11",
        ext: "mp4",
      })
      .then(setVideoPreview);
  }, [
    settings.music_root,
    settings.video_root,
    settings.audio_template,
    settings.video_template,
  ]);

  useEffect(() => {
    if (activeSection === "gamemode") {
      api.listRunningProcesses().then(setRunningProcs);
    }
    if (activeSection === "components" && extractors.length === 0) {
      api.listSupportedSites().then(setExtractors).catch(() => {});
    }
  }, [activeSection]);

  const navItems = [
    { id: "folders", label: "Pastas e organização", icon: FolderOpen },
    { id: "network", label: "Rede e banda", icon: Gauge },
    { id: "gamemode", label: "Modo jogo", icon: Gamepad2 },
    { id: "accounts", label: "Contas", icon: KeyRound },
    { id: "presets", label: "Padrões e presets", icon: Sliders },
    { id: "appearance", label: "Aparência", icon: Palette },
    { id: "components", label: "Componentes", icon: Cpu },
    { id: "advanced", label: "Avançado", icon: Terminal },
  ];

  const pickRootFolder = async (key: "music_root" | "video_root") => {
    const dir = await openDialog({
      directory: true,
      multiple: false,
      defaultPath: settings[key],
    });
    if (typeof dir === "string" && dir.trim()) {
      await updateSettings({ [key]: dir });
    }
  };

  return (
    <div
      style={{
        height: "100%",
        overflowY: "auto",
        padding: "18px 24px 28px",
        position: "relative",
        zIndex: 2,
      }}
    >
      <div
        style={{
          maxWidth: 1020,
          margin: "0 auto",
          display: "grid",
          gridTemplateColumns: "220px 1fr",
          gap: 18,
          alignItems: "start",
        }}
      >
        {/* Section Navigation */}
        <nav
          className="op-panel"
          style={{
            padding: 8,
            display: "flex",
            flexDirection: "column",
            gap: 3,
            position: "sticky",
            top: 0,
          }}
        >
          {navItems.map((item) => {
            const Icon = item.icon;
            const active = activeSection === item.id;
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => setActiveTab("settings", item.id)}
                style={{
                  height: 36,
                  padding: "0 12px",
                  borderRadius: 7,
                  border: "none",
                  background: active ? "var(--accent)" : "transparent",
                  color: active ? "var(--on-accent)" : "var(--text-2)",
                  fontFamily: "var(--font-sans)",
                  fontSize: 13,
                  fontVariationSettings: active
                    ? '"wdth" 108, "wght" 730'
                    : '"wdth" 100, "wght" 500',
                  display: "flex",
                  alignItems: "center",
                  gap: 9,
                  cursor: "pointer",
                  textAlign: "left",
                }}
              >
                <Icon size={15} />
                {item.label}
              </button>
            );
          })}
        </nav>

        {/* Active Section Body */}
        <div className="op-panel" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 18 }}>
          {/* 1. Pastas e organização */}
          {activeSection === "folders" && (
            <>
              <h2 className="font-display" style={{ margin: 0, fontSize: 18 }}>
                Pastas e organização
              </h2>

              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>Raiz de Música</span>
                <PathPreview
                  path={settings.music_root}
                  onClick={() => pickRootFolder("music_root")}
                />
              </div>

              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>Raiz de Vídeos</span>
                <PathPreview
                  path={settings.video_root}
                  onClick={() => pickRootFolder("video_root")}
                />
              </div>

              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>
                  Chips de template (clique para inserir no template de vídeo ou áudio)
                </span>
                <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                  {TEMPLATE_TOKENS.map((tk) => (
                    <Chip
                      key={tk.token}
                      onClick={() =>
                        updateSettings({
                          video_template: `${settings.video_template}\\${tk.token}`,
                        })
                      }
                      title={`Inserir ${tk.token}`}
                    >
                      [{tk.label}]
                    </Chip>
                  ))}
                </div>
              </div>

              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <label style={{ fontSize: 13, fontWeight: 600 }}>Template de Áudio</label>
                <input
                  className="op-input font-mono-num"
                  value={settings.audio_template}
                  onChange={(e) => updateSettings({ audio_template: e.target.value })}
                  style={{ height: 36, padding: "0 12px", fontSize: 13 }}
                />
                <span className="font-mono-num" style={{ fontSize: 11.5, color: "var(--text-2)" }}>
                  Prévia: {audioPreview}
                </span>
              </div>

              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <label style={{ fontSize: 13, fontWeight: 600 }}>Template de Vídeo</label>
                <input
                  className="op-input font-mono-num"
                  value={settings.video_template}
                  onChange={(e) => updateSettings({ video_template: e.target.value })}
                  style={{ height: 36, padding: "0 12px", fontSize: 13 }}
                />
                <span className="font-mono-num" style={{ fontSize: 11.5, color: "var(--text-2)" }}>
                  Prévia: {videoPreview}
                </span>
              </div>

              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>Se o arquivo já existir</span>
                <Segmented<CollisionPolicy>
                  id="collision-policy"
                  value={settings.collision_policy}
                  onChange={(v) => updateSettings({ collision_policy: v })}
                  options={[
                    { value: "keep_both", label: "Manter os dois (2)" },
                    { value: "overwrite", label: "Substituir" },
                    { value: "skip", label: "Pular" },
                  ]}
                />
              </div>
            </>
          )}

          {/* 2. Rede e banda */}
          {activeSection === "network" && (
            <>
              <h2 className="font-display" style={{ margin: 0, fontSize: 18 }}>
                Rede e banda
              </h2>

              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>Perfil de velocidade global</span>
                <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                  {(
                    [
                      ["unlimited", "Sem limite (∞)"],
                      ["balanced", "Equilibrado (60% do pico)"],
                      ["light", "Leve (2 MB/s)"],
                      ["custom", "Personalizado"],
                    ] as [BandwidthProfile, string][]
                  ).map(([prof, lbl]) => (
                    <Chip
                      key={prof}
                      active={settings.bandwidth_profile === prof}
                      onClick={async () => {
                        await api.setLimit(prof, settings.custom_limit_bps);
                        await updateSettings({ bandwidth_profile: prof });
                      }}
                    >
                      {lbl}
                    </Chip>
                  ))}
                </div>
              </div>

              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <div style={{ display: "flex", justifyContent: "space-between", fontSize: 13 }}>
                  <span>Limite personalizado (escala logarítmica):</span>
                  <strong className="font-mono-num">
                    {formatDualBandwidth(settings.custom_limit_bps, settings.speed_unit)}
                  </strong>
                </div>
                <Slider
                  min={0}
                  max={100}
                  step={0.5}
                  value={[bpsToLogSlider(settings.custom_limit_bps)]}
                  onValueChange={async ([v]) => {
                    const bps = logSliderToBps(v);
                    await api.setLimit("custom", bps);
                    await updateSettings({
                      bandwidth_profile: "custom",
                      custom_limit_bps: bps,
                    });
                  }}
                />
                <div className="font-mono-num" style={{ fontSize: 11.5, color: "var(--text-3)" }}>
                  Pico medido (7 dias):{" "}
                  {limiterStats.peak_7d_bps > 0
                    ? formatDualBandwidth(limiterStats.peak_7d_bps, settings.speed_unit)
                    : "ainda não medido"}
                  {limiterStats.proxy_port
                    ? ` · Proxy local ativo (127.0.0.1:${limiterStats.proxy_port}, SO_RCVBUF 256 KB)`
                    : " · Modo compatível"}
                </div>
              </div>

              <div style={{ display: "flex", gap: 16, alignItems: "center", flexWrap: "wrap" }}>
                <span style={{ fontSize: 13 }}>Unidade principal:</span>
                <Segmented<SpeedUnit>
                  id="speed-unit"
                  value={settings.speed_unit}
                  onChange={(u) => updateSettings({ speed_unit: u })}
                  options={[
                    { value: "bytes_per_sec", label: "MB/s" },
                    { value: "bits_per_sec", label: "Mbps" },
                  ]}
                />
              </div>

              <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 10 }}>
                <div>
                  <label style={{ fontSize: 12, color: "var(--text-2)" }}>Global (1–6)</label>
                  <input
                    type="number"
                    min={1}
                    max={6}
                    className="op-input font-mono-num"
                    value={settings.global_concurrency}
                    onChange={(e) => {
                      const n = Math.max(1, Math.min(6, Number(e.target.value) || 3));
                      api.setConcurrency(n);
                      updateSettings({ global_concurrency: n });
                    }}
                    style={{ width: "100%", height: 34, padding: "0 10px", marginTop: 4 }}
                  />
                </div>
                {["youtube", "instagram", "tiktok"].map((host) => (
                  <div key={host}>
                    <label style={{ fontSize: 12, color: "var(--text-2)" }}>
                      Por host ({host})
                    </label>
                    <input
                      type="number"
                      min={1}
                      max={4}
                      className="op-input font-mono-num"
                      value={settings.host_concurrency[host] ?? 2}
                      onChange={(e) =>
                        updateSettings({
                          host_concurrency: {
                            ...settings.host_concurrency,
                            [host]: Math.max(1, Math.min(4, Number(e.target.value) || 1)),
                          },
                        })
                      }
                      style={{ width: "100%", height: 34, padding: "0 10px", marginTop: 4 }}
                    />
                  </div>
                ))}
              </div>

              <Toggle
                checked={settings.monitor_clipboard_background}
                onChange={(v) => updateSettings({ monitor_clipboard_background: v })}
                label="Monitorar área de transferência mesmo em segundo plano (desligado por padrão)"
              />
            </>
          )}

          {/* 3. Modo jogo */}
          {activeSection === "gamemode" && (
            <>
              <h2 className="font-display" style={{ margin: 0, fontSize: 18 }}>
                Modo jogo
              </h2>
              <p style={{ margin: 0, fontSize: 13, color: "var(--text-2)" }}>
                Detecta jogos Direct3D, aplicativos em tela cheia ou executáveis da lista abaixo.
                Quando ativo: reduz a banda, baixa concorrência para 1, coloca encodes da Oficina em{" "}
                <code className="font-mono-num">IDLE_PRIORITY_CLASS</code> e congela o shader do
                fundo (0% GPU).
              </p>

              <div style={{ display: "flex", alignItems: "center", gap: 14, flexWrap: "wrap" }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>Controle:</span>
                <Segmented<GameModeOverride>
                  id="gm-override"
                  value={settings.game_mode_override}
                  onChange={async (ovr) => {
                    const st = await api.setGameModeOverride(ovr);
                    setGameMode(st);
                    await updateSettings({ game_mode_override: ovr });
                  }}
                  options={[
                    { value: "auto", label: "Automático" },
                    { value: "force_on", label: "Forçar ligado" },
                    { value: "force_off", label: "Forçar desligado" },
                  ]}
                />
                {gameMode.active && (
                  <span style={{ fontSize: 12, color: "var(--accent-text)", fontWeight: 700 }}>
                    ● Ativo agora ({gameMode.reason || "Modo jogo"})
                  </span>
                )}
              </div>

              <Toggle
                checked={settings.game_mode_auto}
                onChange={(v) => updateSettings({ game_mode_auto: v })}
                label="Ativar automaticamente ao abrir jogo ou aplicativo em tela cheia"
              />

              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>
                  Executáveis que ativam o Modo jogo
                </span>
                <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                  {settings.game_exe_list.map((exe) => (
                    <Chip
                      key={exe}
                      onClick={() =>
                        updateSettings({
                          game_exe_list: settings.game_exe_list.filter((x) => x !== exe),
                        })
                      }
                      title="Clique para remover"
                    >
                      {exe} ×
                    </Chip>
                  ))}
                </div>
                <div style={{ display: "flex", gap: 8 }}>
                  <input
                    className="op-input font-mono-num"
                    placeholder="Ex.: cyberpunk2077.exe"
                    value={newGameExe}
                    onChange={(e) => setNewGameExe(e.target.value)}
                    style={{ height: 34, padding: "0 10px", flex: 1 }}
                  />
                  <Button
                    variant="secondary"
                    onClick={() => {
                      const clean = newGameExe.trim();
                      if (clean && !settings.game_exe_list.includes(clean)) {
                        updateSettings({
                          game_exe_list: [...settings.game_exe_list, clean],
                        });
                        setNewGameExe("");
                      }
                    }}
                  >
                    Adicionar
                  </Button>
                </div>

                {runningProcs.length > 0 && (
                  <div style={{ marginTop: 4 }}>
                    <div style={{ fontSize: 11.5, color: "var(--text-3)", marginBottom: 6 }}>
                      Em execução agora (clique para adicionar):
                    </div>
                    <div style={{ display: "flex", gap: 6, flexWrap: "wrap", maxHeight: 90, overflowY: "auto" }}>
                      {runningProcs
                        .filter(
                          (p) =>
                            !p.toLowerCase().startsWith("svchost") &&
                            !p.toLowerCase().startsWith("system") &&
                            !settings.game_exe_list.includes(p.toLowerCase())
                        )
                        .slice(0, 18)
                        .map((proc) => (
                          <Chip
                            key={proc}
                            onClick={() =>
                              updateSettings({
                                game_exe_list: [...settings.game_exe_list, proc.toLowerCase()],
                              })
                            }
                          >
                            + {proc}
                          </Chip>
                        ))}
                    </div>
                  </div>
                )}
              </div>
            </>
          )}

          {/* 4. Contas e cookies */}
          {activeSection === "accounts" && (
            <>
              <h2 className="font-display" style={{ margin: 0, fontSize: 18 }}>
                Contas e cookies
              </h2>

              <div
                style={{
                  padding: "10px 12px",
                  borderRadius: 8,
                  border: "1px solid var(--warn)",
                  background: "var(--surface-2)",
                  fontSize: 12.5,
                  display: "flex",
                  gap: 10,
                  alignItems: "flex-start",
                }}
              >
                <AlertTriangle size={16} color="var(--warn)" style={{ marginTop: 2, flexShrink: 0 }} />
                <div>
                  <strong style={{ color: "var(--warn)" }}>Aviso sobre YouTube:</strong> Usar conta
                  logada com o yt-dlp pode levar a restrições na conta. Use só quando precisar
                  (restrição de idade, membros), de preferência com uma conta secundária. Chrome e
                  Edge (v127+) usam criptografia app-bound no Windows, então para YouTube prefira{" "}
                  <strong>Firefox</strong> ou importar um <code className="font-mono-num">cookies.txt</code>.
                </div>
              </div>

              <div style={{ fontSize: 12.5, color: "var(--text-2)" }}>
                Cookies funcionam como senha. O Boladão guarda os seus criptografados pelo Windows (DPAPI),
                presos à sua conta de usuário, e só os decifra num arquivo temporário enquanto o yt-dlp roda.
              </div>

              {accountMsg && (
                <div style={{ fontSize: 12.5, color: "var(--accent-text)" }}>{accountMsg}</div>
              )}

              <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
                {ACCOUNT_PLATFORMS.map((plat) => {
                  const acc = settings.accounts[plat.id];
                  const isWindowOpen = openLoginPlatform === plat.id;
                  return (
                    <div
                      key={plat.id}
                      style={{
                        padding: "12px 14px",
                        borderRadius: 8,
                        background: "var(--surface-2)",
                        border: "1px solid var(--line)",
                        display: "flex",
                        alignItems: "center",
                        justifyContent: "space-between",
                        flexWrap: "wrap",
                        gap: 12,
                      }}
                    >
                      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                        <PlatformIcon slug={plat.slug} color={plat.color} size={18} />
                        <div>
                          <div style={{ fontWeight: 700, fontSize: 13.5 }}>{plat.name}</div>
                          <div style={{ fontSize: 11.5, color: "var(--text-2)" }}>
                            {acc
                              ? `Conectado (${
                                  acc.auth_kind === "firefox_browser"
                                    ? "Firefox"
                                    : acc.auth_kind === "imported_cookies_file"
                                    ? `cookies.txt · ${acc.cookie_count} cookies`
                                    : `${acc.cookie_count} cookies`
                                })${
                                  acc.earliest_expiry_ts
                                    ? ` · expira ${new Date(
                                        acc.earliest_expiry_ts * 1000
                                      ).toLocaleDateString("pt-BR")}`
                                    : ""
                                }`
                              : "Não conectado"}
                          </div>
                        </div>
                      </div>

                      <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                        {plat.id === "youtube" && !acc && (
                          <>
                            <Button
                              size="sm"
                              variant="primary"
                              onClick={async () => {
                                await api.connectFirefoxCookies("youtube");
                                await loadSettings();
                                setAccountMsg("YouTube conectado aos cookies do Firefox.");
                              }}
                            >
                              Usar Firefox
                            </Button>
                            <Button
                              size="sm"
                              variant="secondary"
                              onClick={async () => {
                                const f = await openDialog({
                                  multiple: false,
                                  filters: [{ name: "Cookies Netscape", extensions: ["txt"] }],
                                });
                                if (typeof f === "string") {
                                  await api.importCookiesFile("youtube", f);
                                  await loadSettings();
                                  setAccountMsg(
                                    "cookies.txt importado e criptografado. Você já pode apagar o arquivo original."
                                  );
                                }
                              }}
                            >
                              Importar cookies.txt
                            </Button>
                          </>
                        )}

                        {!acc && !isWindowOpen && (
                          <Button
                            size="sm"
                            variant={plat.id === "youtube" ? "ghost" : "primary"}
                            onClick={async () => {
                              try {
                                setAccountMsg(null);
                                await api.loginPlatform(plat.id);
                                setOpenLoginPlatform(plat.id);
                                setAccountMsg(
                                  `Janela aberta para ${plat.name}. Entre na sua conta e clique em 'Concluir login (salvar cookies)'.`
                                );
                              } catch (e: any) {
                                setAccountMsg(String(e));
                              }
                            }}
                          >
                            {plat.id === "youtube" ? "Janela embutida" : "Entrar"}
                          </Button>
                        )}

                        {isWindowOpen && (
                          <Button
                            size="sm"
                            variant="primary"
                            onClick={async () => {
                              try {
                                await api.finishLoginPlatform(plat.id);
                                setOpenLoginPlatform(null);
                                await loadSettings();
                                setAccountMsg(`Conta ${plat.name} conectada.`);
                              } catch (e: any) {
                                setAccountMsg(String(e));
                              }
                            }}
                          >
                            Concluir login (salvar cookies)
                          </Button>
                        )}

                        {acc && (
                          <Button
                            size="sm"
                            variant="warn"
                            onClick={async () => {
                              await api.logoutPlatform(plat.id);
                              setOpenLoginPlatform(null);
                              await loadSettings();
                            }}
                          >
                            Sair
                          </Button>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            </>
          )}

          {/* 5. Padrões e presets */}
          {activeSection === "presets" && (
            <>
              <h2 className="font-display" style={{ margin: 0, fontSize: 18 }}>
                Padrões e presets
              </h2>
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                {presets.map((p) => (
                  <div
                    key={p.id}
                    style={{
                      padding: "10px 14px",
                      borderRadius: 8,
                      background: "var(--surface-2)",
                      border: "1px solid var(--line)",
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                    }}
                  >
                    <div>
                      <div style={{ fontWeight: 700, fontSize: 13.5 }}>
                        {p.name}{" "}
                        {p.builtin && (
                          <span style={{ fontSize: 11, color: "var(--text-3)", fontWeight: 400 }}>
                            (padrão)
                          </span>
                        )}
                      </div>
                      <div style={{ fontSize: 12, color: "var(--text-2)" }}>{p.description}</div>
                    </div>
                    {!p.builtin && (
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={async () => {
                          await api.deleteCustomPreset(p.id);
                          await refreshPresets();
                        }}
                      >
                        <Trash2 size={14} /> Excluir
                      </Button>
                    )}
                  </div>
                ))}
              </div>
            </>
          )}

          {/* 6. Aparência */}
          {activeSection === "appearance" && (
            <>
              <h2 className="font-display" style={{ margin: 0, fontSize: 18 }}>
                Aparência
              </h2>

              <div style={{ display: "grid", gridTemplateColumns: "repeat(5, 1fr)", gap: 10 }}>
                {(
                  [
                    { id: "sangue", name: "Sangue", swA: "#FA023E", swB: "#161616" },
                    { id: "papel", name: "Papel", swA: "#161616", swB: "#F2EEE6" },
                    { id: "acido", name: "Ácido", swA: "#C6FF00", swB: "#121212" },
                    { id: "cobalto", name: "Cobalto", swA: "#2B4BFF", swB: "#0D0F1A" },
                    { id: "breu", name: "Breu", swA: "#000000", swB: "#1A1A1A" },
                  ] as { id: ThemeId; name: string; swA: string; swB: string }[]
                ).map((th) => {
                  const selected = settings.theme === th.id;
                  return (
                    <button
                      key={th.id}
                      type="button"
                      onClick={() => updateSettings({ theme: th.id })}
                      style={{
                        padding: 12,
                        borderRadius: 8,
                        background: "var(--surface-2)",
                        border: selected ? "2px solid var(--accent)" : "1px solid var(--line)",
                        color: "var(--text)",
                        cursor: "pointer",
                        display: "flex",
                        flexDirection: "column",
                        gap: 8,
                        textAlign: "left",
                      }}
                    >
                      <div
                        style={{
                          height: 28,
                          borderRadius: 5,
                          background: `repeating-linear-gradient(-45deg, ${th.swA} 0 10px, ${th.swB} 10px 18px)`,
                          border: "1px solid var(--line)",
                        }}
                      />
                      <span style={{ fontWeight: 700, fontSize: 13 }}>{th.name}</span>
                    </button>
                  );
                })}
              </div>

              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <div style={{ display: "flex", justifyContent: "space-between", fontSize: 13 }}>
                  <span>Intensidade do fundo Op-Art</span>
                  <span className="font-mono-num">
                    {Math.round((settings.shader_intensity ?? 1) * 100)}%
                  </span>
                </div>
                <Slider
                  min={0.1}
                  max={1.0}
                  step={0.05}
                  value={[settings.shader_intensity ?? 1.0]}
                  onValueChange={([v]) => updateSettings({ shader_intensity: v })}
                />
              </div>

              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <div style={{ display: "flex", justifyContent: "space-between", fontSize: 13 }}>
                  <span>Densidade das faixas topográficas</span>
                  <span className="font-mono-num">
                    {settings.shader_stripe_density.toFixed(2)}x
                  </span>
                </div>
                <Slider
                  min={0.7}
                  max={1.4}
                  step={0.05}
                  value={[settings.shader_stripe_density]}
                  onValueChange={([v]) => updateSettings({ shader_stripe_density: v })}
                />
              </div>

              <Toggle
                checked={settings.reduce_motion}
                onChange={(v) => updateSettings({ reduce_motion: v })}
                label="Reduzir movimento (congela o shader num quadro estático — 0% GPU)"
              />
            </>
          )}

          {/* 7. Componentes */}
          {activeSection === "components" && (
            <>
              <h2 className="font-display" style={{ margin: 0, fontSize: 18 }}>
                Componentes embutidos
              </h2>

              <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 10 }}>
                {[
                  { label: "yt-dlp (unpackaged)", ver: binsStatus.ytdlp_version },
                  { label: "Deno JS Runtime", ver: binsStatus.deno_version },
                  { label: "FFmpeg + FFprobe", ver: binsStatus.ffmpeg_version },
                ].map((c) => (
                  <div
                    key={c.label}
                    style={{
                      padding: 12,
                      borderRadius: 8,
                      background: "var(--surface-2)",
                      border: "1px solid var(--line)",
                    }}
                  >
                    <div style={{ fontSize: 11.5, color: "var(--text-2)" }}>{c.label}</div>
                    <div className="font-mono-num" style={{ fontSize: 16, fontWeight: 700, marginTop: 4 }}>
                      {c.ver}
                    </div>
                  </div>
                ))}
              </div>

              <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
                <Segmented<string>
                  id="ytdlp-channel"
                  value={settings.ytdlp_channel}
                  onChange={(ch) => updateSettings({ ytdlp_channel: ch })}
                  options={[
                    { value: "stable", label: "Canal Stable" },
                    { value: "nightly", label: "Canal Nightly" },
                  ]}
                />
                <Button
                  variant="primary"
                  disabled={updatingYtdlp}
                  onClick={async () => {
                    setUpdatingYtdlp(true);
                    try {
                      await api.updateYtdlp(settings.ytdlp_channel);
                      setBinsStatus(await api.getBinsStatus());
                    } finally {
                      setUpdatingYtdlp(false);
                    }
                  }}
                >
                  <RefreshCw size={14} />{" "}
                  {updatingYtdlp ? "Atualizando yt-dlp..." : "Atualizar yt-dlp"}
                </Button>

                {binsStatus.ytdlp_previous_version && (
                  <Button
                    variant="secondary"
                    onClick={async () => {
                      await api.rollbackYtdlp();
                      setBinsStatus(await api.getBinsStatus());
                    }}
                  >
                    <RotateCcw size={14} /> Voltar para {binsStatus.ytdlp_previous_version}
                  </Button>
                )}

                <Button
                  variant="secondary"
                  onClick={async () => {
                    setDiagOutput("Executando yt-dlp -v...");
                    const out = await api.runYtdlpDiagnostic();
                    setDiagOutput(out);
                  }}
                >
                  Testar motores (yt-dlp -v)
                </Button>
              </div>

              {diagOutput && (
                <pre
                  className="font-mono-num selectable"
                  style={{
                    margin: 0,
                    padding: 10,
                    borderRadius: 8,
                    background: "var(--bg)",
                    border: "1px solid var(--line)",
                    fontSize: 11,
                    color: "var(--text-2)",
                    whiteSpace: "pre-wrap",
                  }}
                >
                  {diagOutput}
                </pre>
              )}

              {/* Detected GPU Encoders & FFmpeg Filters */}
              <div>
                <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 6 }}>
                  Encoders de GPU validados (teste real de 0,1 s)
                </div>
                <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                  {binsStatus.capabilities.hw_encoders.length > 0 ? (
                    binsStatus.capabilities.hw_encoders.map((enc) => (
                      <Chip key={enc} active>
                        {enc}
                      </Chip>
                    ))
                  ) : (
                    <span style={{ fontSize: 12, color: "var(--text-2)" }}>
                      Nenhum encoder de hardware ativo nesta máquina (usando CPU libx264/libx265/libsvtav1).
                    </span>
                  )}
                </div>
              </div>

              <div>
                <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 6 }}>
                  Filtros FFmpeg detectados
                </div>
                <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                  {[
                    ["soxr (alta precisão)", binsStatus.capabilities.has_soxr],
                    ["loudnorm (EBU R128)", binsStatus.capabilities.has_loudnorm],
                    ["rubberband (pitch)", binsStatus.capabilities.has_rubberband],
                    ["libass (subtitles)", binsStatus.capabilities.has_libass],
                    ["whisper.cpp", binsStatus.capabilities.has_whisper],
                    ["palettegen (GIF)", binsStatus.capabilities.has_palettegen],
                  ].map(([name, ok]) => (
                    <Chip key={String(name)} active={Boolean(ok)} disabled={!ok}>
                      {String(name)}
                    </Chip>
                  ))}
                </div>
              </div>

              {/* Searchable Supported Sites (`yt-dlp --list-extractors`) */}
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <div style={{ fontSize: 13, fontWeight: 600 }}>
                  Sites suportados ({extractors.length} extratores)
                </div>
                <input
                  className="op-input"
                  placeholder="Pesquisar extrator (ex.: youtube, bandcamp, globo, twitch)..."
                  value={extractorSearch}
                  onChange={(e) => setExtractorSearch(e.target.value)}
                  style={{ height: 34, padding: "0 10px", fontSize: 12.5 }}
                />
                <div
                  className="font-mono-num"
                  style={{
                    maxHeight: 150,
                    overflowY: "auto",
                    padding: 8,
                    borderRadius: 6,
                    background: "var(--surface-2)",
                    border: "1px solid var(--line)",
                    fontSize: 11.5,
                    display: "flex",
                    flexWrap: "wrap",
                    gap: 6,
                  }}
                >
                  {extractors
                    .filter((e) =>
                      e.toLowerCase().includes(extractorSearch.trim().toLowerCase())
                    )
                    .slice(0, 120)
                    .map((ex) => (
                      <span
                        key={ex}
                        style={{
                          padding: "2px 6px",
                          borderRadius: 4,
                          background: "var(--bg)",
                          border: "1px solid var(--line)",
                        }}
                      >
                        {ex}
                      </span>
                    ))}
                </div>
              </div>
            </>
          )}

          {/* 8. Avançado */}
          {activeSection === "advanced" && (
            <>
              <h2 className="font-display" style={{ margin: 0, fontSize: 18 }}>
                Avançado
              </h2>

              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <label style={{ fontSize: 13, fontWeight: 600 }}>
                  Argumentos extras globais do yt-dlp
                </label>
                <input
                  className="op-input font-mono-num"
                  placeholder="Ex.: --geo-bypass --socket-timeout 20"
                  value={extraArgsText}
                  onChange={(e) => setExtraArgsText(e.target.value)}
                  onBlur={() =>
                    updateSettings({
                      global_extra_ytdlp_args: extraArgsText
                        .split(/\s+/)
                        .map((s) => s.trim())
                        .filter(Boolean),
                    })
                  }
                  style={{ height: 36, padding: "0 12px", fontSize: 12.5 }}
                />
                <span style={{ fontSize: 11.5, color: "var(--text-3)" }}>
                  Flags de execução ou saída (<code className="font-mono-num">--exec</code>,{" "}
                  <code className="font-mono-num">-o</code>,{" "}
                  <code className="font-mono-num">--proxy</code>) são bloqueadas por segurança.
                </span>
              </div>

              <div style={{ display: "flex", gap: 10, flexWrap: "wrap" }}>
                <Button variant="secondary" onClick={() => api.openDataFolder("roaming")}>
                  <FolderOpen size={14} /> Abrir pasta de configuração (%APPDATA%\Boladao)
                </Button>
                <Button variant="secondary" onClick={() => api.openDataFolder("local")}>
                  <FolderOpen size={14} /> Abrir pasta de binários e cache (%LOCALAPPDATA%\Boladao)
                </Button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
};
