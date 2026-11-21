import React, { useEffect, useState } from "react";
import {
  Activity,
  Copy,
  ExternalLink,
  Film,
  FolderOpen,
  GripVertical,
  Music,
  RefreshCw,
  Search,
  Trash2,
  Wrench,
} from "lucide-react";
import { api, LibraryEntry, MediaMode } from "../../lib/ipc";
import { formatBytes, formatDuration } from "../../lib/format";
import { useAppStore } from "../../stores/appStore";
import { Button, Chip, Segmented, Toggle } from "../../ui/Primitives";

export const LibraryTab: React.FC = () => {
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const setPendingUrlToLoad = useAppStore((s) => s.setPendingUrlToLoad);

  const [entries, setEntries] = useState<LibraryEntry[]>([]);
  const [search, setSearch] = useState("");
  const [modeFilter, setModeFilter] = useState<"all" | MediaMode>("all");
  const [selected, setSelected] = useState<LibraryEntry | null>(null);
  const [loadingSpec, setLoadingSpec] = useState(false);
  const [runningWs, setRunningWs] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [copiedCmd, setCopiedCmd] = useState(false);

  // Quick standalone workshop controls for selected item
  const [wsLoudnorm, setWsLoudnorm] = useState<number | null>(-14);
  const [wsTrimSilence, setWsTrimSilence] = useState(false);
  const [wsOpen, setWsOpen] = useState(false);

  const reload = async () => {
    const list = await api.queryLibrary({
      search: search.trim() || null,
      mode: modeFilter === "all" ? null : modeFilter,
    });
    setEntries(list);
    if (selected) {
      const updated = list.find((x) => x.id === selected.id) || null;
      setSelected(updated);
    }
  };

  useEffect(() => {
    const t = setTimeout(reload, 120);
    return () => clearTimeout(t);
  }, [search, modeFilter]);

  const handleGenerateSpectrogram = async (entry: LibraryEntry) => {
    setLoadingSpec(true);
    try {
      const updated = await api.generateSpectrogram(entry.id);
      setSelected(updated);
      setEntries((prev) => prev.map((x) => (x.id === updated.id ? updated : x)));
    } finally {
      setLoadingSpec(false);
    }
  };

  const handleStandaloneWorkshop = async (entry: LibraryEntry) => {
    setRunningWs(true);
    try {
      const nextSpec = {
        ...entry.spec,
        workshop: {
          ...entry.spec.workshop,
          loudnorm_lufs: wsLoudnorm,
          trim_silence: wsTrimSilence,
        },
      };
      const updated = await api.runWorkshop(entry.id, nextSpec);
      setSelected(updated);
      setEntries((prev) => prev.map((x) => (x.id === updated.id ? updated : x)));
      setWsOpen(false);
    } finally {
      setRunningWs(false);
    }
  };

  return (
    <div
      style={{
        height: "100%",
        overflowY: "auto",
        padding: "18px 24px 24px",
        position: "relative",
        zIndex: 2,
      }}
    >
      <div
        style={{
          maxWidth: 1020,
          margin: "0 auto",
          display: "grid",
          gridTemplateColumns: selected ? "1fr 370px" : "1fr",
          gap: 16,
          alignItems: "start",
        }}
      >
        {/* Left Column: Search + Filters + History List */}
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div
            className="op-panel"
            style={{
              padding: "10px 14px",
              display: "flex",
              alignItems: "center",
              gap: 10,
              flexWrap: "wrap",
            }}
          >
            <div style={{ position: "relative", flex: 1, minWidth: 200 }}>
              <Search
                size={15}
                style={{
                  position: "absolute",
                  left: 10,
                  top: "50%",
                  transform: "translateY(-50%)",
                  color: "var(--text-3)",
                }}
              />
              <input
                className="op-input"
                placeholder="Buscar por título, canal, artista ou plataforma..."
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                style={{ width: "100%", height: 34, padding: "0 10px 0 32px", fontSize: 13 }}
              />
            </div>

            <Segmented<"all" | MediaMode>
              id="lib-filter-mode"
              value={modeFilter}
              onChange={setModeFilter}
              options={[
                { value: "all", label: "Tudo" },
                { value: "audio", label: "Áudio" },
                { value: "video", label: "Vídeo" },
              ]}
            />
          </div>

          {entries.length === 0 ? (
            <div
              className="op-panel"
              style={{ padding: "56px 24px", textAlign: "center", color: "var(--text-2)" }}
            >
              Nenhum arquivo na Biblioteca ainda.
            </div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              {entries.map((entry) => {
                const isSel = selected?.id === entry.id;
                return (
                  <div
                    key={entry.id}
                    onClick={() => setSelected(entry)}
                    className="op-panel"
                    style={{
                      padding: "10px 14px",
                      borderColor: isSel ? "var(--accent)" : "var(--line)",
                      opacity: entry.file_exists ? 1 : 0.55,
                      cursor: "pointer",
                      display: "flex",
                      flexDirection: "column",
                      gap: 8,
                    }}
                  >
                    <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                      <div
                        title={
                          entry.file_exists
                            ? "Arraste o arquivo para fora do app"
                            : "Arquivo não encontrado"
                        }
                        onMouseDown={(e) => {
                          if (entry.file_exists) {
                            e.stopPropagation();
                            api.startFileDrag(entry.primary_path);
                          }
                        }}
                        style={{ color: "var(--text-3)", cursor: "grab" }}
                      >
                        <GripVertical size={14} />
                      </div>

                      <div
                        style={{
                          width: 46,
                          height: 46,
                          borderRadius: 6,
                          overflow: "hidden",
                          background: "var(--surface-2)",
                          border: "1px solid var(--line)",
                          flexShrink: 0,
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "center",
                        }}
                      >
                        {entry.thumb_url ? (
                          <img
                            src={entry.thumb_url}
                            alt=""
                            style={{ width: "100%", height: "100%", objectFit: "cover" }}
                          />
                        ) : entry.mode === "audio" ? (
                          <Music size={18} color="var(--text-3)" />
                        ) : (
                          <Film size={18} color="var(--text-3)" />
                        )}
                      </div>

                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                          <span
                            style={{
                              fontWeight: 650,
                              fontSize: 13.5,
                              overflow: "hidden",
                              textOverflow: "ellipsis",
                              whiteSpace: "nowrap",
                              flex: 1,
                            }}
                          >
                            {entry.title}
                          </span>
                          <span
                            className="font-mono-num"
                            style={{
                              fontSize: 11,
                              padding: "1px 6px",
                              borderRadius: 4,
                              background: "var(--surface-2)",
                              border: "1px solid var(--line)",
                              color: "var(--text-2)",
                            }}
                          >
                            {entry.format_badge}
                          </span>
                        </div>

                        <div style={{ fontSize: 12, color: "var(--text-2)", marginTop: 2 }}>
                          {entry.channel_or_artist} · {entry.platform_name} ·{" "}
                          <span className="font-mono-num">
                            {formatBytes(entry.details.file_size_bytes)}
                          </span>
                          {entry.details.integrated_lufs != null && (
                            <span className="font-mono-num">
                              {" "}
                              · {entry.details.integrated_lufs.toFixed(1)} LUFS
                            </span>
                          )}
                          {!entry.file_exists && (
                            <span style={{ color: "var(--warn)", marginLeft: 8 }}>
                              · Arquivo não encontrado
                            </span>
                          )}
                        </div>
                      </div>
                    </div>

                    {/* Audio Waveform (`showwavespic` 1200x96 PNG — Section 8.4) */}
                    {entry.mode === "audio" && entry.waveform_url && (
                      <div
                        style={{
                          height: 36,
                          borderRadius: 6,
                          background: "var(--surface-2)",
                          border: "1px solid var(--line)",
                          overflow: "hidden",
                          padding: "2px 6px",
                        }}
                      >
                        <img
                          src={entry.waveform_url}
                          alt="Forma de onda do áudio"
                          style={{ width: "100%", height: "100%", objectFit: "fill" }}
                        />
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Right Column: Deep ffprobe + LUFS + Spectrogram + Workshop Inspector */}
        {selected && (
          <div
            className="op-panel"
            style={{
              padding: 16,
              display: "flex",
              flexDirection: "column",
              gap: 12,
              position: "sticky",
              top: 0,
            }}
          >
            <div style={{ fontWeight: 750, fontSize: 15, lineHeight: 1.3 }}>
              {selected.title}
            </div>

            {/* Technical ffprobe Table */}
            <div
              className="font-mono-num"
              style={{
                padding: 10,
                borderRadius: 8,
                background: "var(--surface-2)",
                border: "1px solid var(--line)",
                fontSize: 11.5,
                display: "grid",
                gridTemplateColumns: "110px 1fr",
                rowGap: 5,
              }}
            >
              <span style={{ color: "var(--text-3)" }}>Contêiner:</span>
              <span>{selected.details.container_format || "—"}</span>
              {selected.details.video_codec && (
                <>
                  <span style={{ color: "var(--text-3)" }}>Vídeo:</span>
                  <span>
                    {selected.details.video_codec}{" "}
                    {selected.details.width && selected.details.height
                      ? `(${selected.details.width}×${selected.details.height}${
                          selected.details.fps ? ` @ ${selected.details.fps}fps` : ""
                        })`
                      : ""}
                  </span>
                </>
              )}
              {selected.details.audio_codec && (
                <>
                  <span style={{ color: "var(--text-3)" }}>Áudio:</span>
                  <span>
                    {selected.details.audio_codec}
                    {selected.details.sample_rate_hz
                      ? ` · ${(selected.details.sample_rate_hz / 1000).toFixed(1)} kHz`
                      : ""}
                    {selected.details.bits_per_sample
                      ? ` · ${selected.details.bits_per_sample}-bit`
                      : ""}
                    {selected.details.channels ? ` · ${selected.details.channels}ch` : ""}
                  </span>
                </>
              )}
              <span style={{ color: "var(--text-3)" }}>Duração:</span>
              <span>{formatDuration(selected.details.duration_s)}</span>
              <span style={{ color: "var(--text-3)" }}>Bitrate:</span>
              <span>
                {selected.details.overall_bitrate_kbps
                  ? `${selected.details.overall_bitrate_kbps} kbps`
                  : "—"}
              </span>
              <span style={{ color: "var(--text-3)" }}>Loudness:</span>
              <span>
                {selected.details.integrated_lufs != null
                  ? `${selected.details.integrated_lufs.toFixed(1)} LUFS`
                  : "—"}
              </span>
              {selected.details.cover_dimensions && (
                <>
                  <span style={{ color: "var(--text-3)" }}>Capa:</span>
                  <span>
                    {selected.details.cover_dimensions[0]}×
                    {selected.details.cover_dimensions[1]} px
                  </span>
                </>
              )}
            </div>

            {/* On-demand Spectrogram + ~16 kHz lossy source shelf-cut note */}
            {selected.file_exists && (
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                {selected.spectrogram_url ? (
                  <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                    <img
                      src={selected.spectrogram_url}
                      alt="Espectrograma de frequências"
                      style={{
                        width: "100%",
                        borderRadius: 6,
                        border: "1px solid var(--line)",
                      }}
                    />
                    {selected.details.lossy_cutoff_note && (
                      <div
                        style={{
                          padding: "6px 10px",
                          borderRadius: 6,
                          border: "1px solid var(--warn)",
                          color: "var(--warn)",
                          fontSize: 12,
                        }}
                      >
                        {selected.details.lossy_cutoff_note}
                      </div>
                    )}
                  </div>
                ) : (
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={loadingSpec}
                    onClick={() => handleGenerateSpectrogram(selected)}
                  >
                    <Activity size={13} />{" "}
                    {loadingSpec ? "Gerando espectrograma..." : "Ver espectrograma de qualidade"}
                  </Button>
                )}
              </div>
            )}

            <div
              className="font-mono-num selectable"
              style={{
                fontSize: 11,
                color: "var(--text-2)",
                wordBreak: "break-all",
                padding: "6px 8px",
                borderRadius: 6,
                background: "var(--surface-2)",
              }}
            >
              {selected.primary_path}
            </div>

            {/* Action Buttons */}
            <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
              {selected.file_exists && (
                <>
                  <Button
                    size="sm"
                    variant="primary"
                    onClick={() => api.openFile(selected.primary_path)}
                  >
                    <ExternalLink size={13} /> Abrir
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => api.revealInExplorer(selected.primary_path)}
                  >
                    <FolderOpen size={13} /> Mostrar na pasta
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => setWsOpen((v) => !v)}
                  >
                    <Wrench size={13} /> Abrir na oficina
                  </Button>
                </>
              )}
              <Button
                size="sm"
                variant="secondary"
                onClick={() => {
                  setPendingUrlToLoad(selected.url);
                  setActiveTab("download");
                }}
              >
                <RefreshCw size={13} />{" "}
                {selected.is_playlist_origin ? "Sincronizar / Outro formato" : "Baixar em outro formato"}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                onClick={async () => {
                  if (selected.equivalent_cmd) {
                    await navigator.clipboard.writeText(selected.equivalent_cmd);
                    setCopiedCmd(true);
                    setTimeout(() => setCopiedCmd(false), 1500);
                  }
                }}
              >
                <Copy size={13} /> {copiedCmd ? "Copiado!" : "Comando"}
              </Button>
            </div>

            {/* Inline Standalone Workshop */}
            {wsOpen && selected.file_exists && (
              <div
                style={{
                  padding: 10,
                  borderRadius: 8,
                  background: "var(--surface-2)",
                  border: "1px solid var(--line)",
                  display: "flex",
                  flexDirection: "column",
                  gap: 8,
                }}
              >
                <div style={{ fontSize: 12, fontWeight: 700 }}>Processar arquivo na Oficina</div>
                <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                  {[-14, -16, -23].map((lufs) => (
                    <Chip
                      key={lufs}
                      active={wsLoudnorm === lufs}
                      onClick={() => setWsLoudnorm(lufs)}
                    >
                      Normalizar {lufs} LUFS
                    </Chip>
                  ))}
                </div>
                <Toggle
                  checked={wsTrimSilence}
                  onChange={setWsTrimSilence}
                  label="Cortar silêncio inicial e final"
                />
                <Button
                  size="sm"
                  variant="primary"
                  disabled={runningWs}
                  onClick={() => handleStandaloneWorkshop(selected)}
                >
                  {runningWs ? "Processando no FFmpeg..." : "Aplicar agora"}
                </Button>
              </div>
            )}

            {/* Delete with confirmation */}
            <div style={{ borderTop: "1px solid var(--line)", paddingTop: 10 }}>
              {confirmDeleteId === selected.id ? (
                <div style={{ display: "flex", gap: 6, alignItems: "center", justifyContent: "space-between" }}>
                  <span style={{ fontSize: 12, color: "var(--warn)" }}>
                    Apagar também do disco?
                  </span>
                  <div style={{ display: "flex", gap: 6 }}>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={async () => {
                        await api.deleteLibraryEntry(selected.id, false);
                        setConfirmDeleteId(null);
                        setSelected(null);
                        reload();
                      }}
                    >
                      Só histórico
                    </Button>
                    <Button
                      size="sm"
                      variant="warn"
                      onClick={async () => {
                        await api.deleteLibraryEntry(selected.id, true);
                        setConfirmDeleteId(null);
                        setSelected(null);
                        reload();
                      }}
                    >
                      Apagar arquivo
                    </Button>
                  </div>
                </div>
              ) : (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setConfirmDeleteId(selected.id)}
                >
                  <Trash2 size={13} /> Remover / Apagar arquivo
                </Button>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
