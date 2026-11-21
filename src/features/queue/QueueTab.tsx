import React, { useMemo, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import * as ContextMenu from "@radix-ui/react-context-menu";
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  Copy,
  ExternalLink,
  FolderOpen,
  GripVertical,
  Minus,
  Pause,
  Play,
  Plus,
  RotateCcw,
  Trash2,
  X,
} from "lucide-react";
import { api, Job } from "../../lib/ipc";
import {
  formatBytes,
  formatEta,
  formatSpeedPrimary,
} from "../../lib/format";
import { useAppStore } from "../../stores/appStore";
import { useQueueStore } from "../../stores/queueStore";
import { useSettingsStore } from "../../stores/settingsStore";
import {
  Button,
  PlatformIcon,
  SPRING_UI,
  StripeProgress,
} from "../../ui/Primitives";

function formatJobBadge(job: Job): string {
  if (job.spec.mode === "audio") {
    return job.spec.audio_format.toUpperCase();
  }
  const h = job.spec.max_height ? `${job.spec.max_height}p ` : "";
  const c =
    job.spec.video_profile === "compatible"
      ? "MP4"
      : job.spec.video_profile === "max_quality"
      ? "MKV"
      : job.spec.custom_container.toUpperCase();
  return `${h}${c}`;
}

/** Animated stroke-dashoffset checkmark when a download finishes (Section 13.8). */
const AnimatedDoneCheck: React.FC = () => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <motion.path
      d="M5 13l4 4L19 7"
      stroke="var(--ok)"
      strokeWidth="2.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      initial={{ pathLength: 0 }}
      animate={{ pathLength: 1 }}
      transition={{ duration: 0.35, ease: "easeOut" }}
    />
  </svg>
);

export const QueueTab: React.FC = () => {
  const jobs = useQueueStore((s) => s.jobs);
  const selectedIds = useQueueStore((s) => s.selectedIds);
  const expandedId = useQueueStore((s) => s.expandedId);
  const toggleSelect = useQueueStore((s) => s.toggleSelect);
  const setExpandedId = useQueueStore((s) => s.setExpandedId);

  const settings = useSettingsStore((s) => s.settings);
  const updateSettings = useSettingsStore((s) => s.updateSettings);
  const setActiveTab = useAppStore((s) => s.setActiveTab);
  const setPendingUrlToLoad = useAppStore((s) => s.setPendingUrlToLoad);

  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const sections = useMemo(() => {
    const active: Job[] = [];
    const waiting: Job[] = [];
    const doneToday: Job[] = [];
    const failed: Job[] = [];

    const startOfDay = new Date();
    startOfDay.setHours(0, 0, 0, 0);

    for (const j of jobs) {
      if (
        j.state === "downloading" ||
        j.state === "probing" ||
        j.state === "postprocessing" ||
        j.state === "workshop" ||
        j.state === "waiting_premiere"
      ) {
        active.push(j);
      } else if (j.state === "queued" || j.state === "paused") {
        waiting.push(j);
      } else if (j.state === "failed") {
        failed.push(j);
      } else if (j.state === "done" || j.state === "canceled") {
        const fin = j.finished_at ? new Date(j.finished_at) : new Date(j.created_at);
        if (fin >= startOfDay || doneToday.length < 30) {
          doneToday.push(j);
        }
      }
    }
    return { active, waiting, doneToday, failed };
  }, [jobs]);

  const playlistGroups = useMemo(() => {
    const map = new Map<string, { title: string; total: number; done: number; pctSum: number }>();
    for (const j of jobs) {
      if (!j.group_id) continue;
      const prev = map.get(j.group_id) || {
        title: j.group_title || "Playlist",
        total: 0,
        done: 0,
        pctSum: 0,
      };
      prev.total += 1;
      if (j.state === "done") prev.done += 1;
      prev.pctSum += j.progress.pct;
      map.set(j.group_id, prev);
    }
    return map;
  }, [jobs]);

  const changeConcurrency = async (delta: number) => {
    const next = Math.max(1, Math.min(6, settings.global_concurrency + delta));
    await api.setConcurrency(next);
    await updateSettings({ global_concurrency: next });
  };

  const handleDropReorder = async (targetId: string) => {
    if (!draggingId || draggingId === targetId) return;
    const ids = jobs.map((j) => j.id);
    const fromIdx = ids.indexOf(draggingId);
    const toIdx = ids.indexOf(targetId);
    if (fromIdx < 0 || toIdx < 0) return;
    ids.splice(fromIdx, 1);
    ids.splice(toIdx, 0, draggingId);
    setDraggingId(null);
    await api.reorder(ids);
  };

  const copyCommand = async (job: Job) => {
    const cmd =
      job.equivalent_cmd ||
      (await api.equivalentCommand(job.id).catch(() => ""));
    if (cmd) {
      await navigator.clipboard.writeText(cmd);
      setCopiedId(job.id);
      setTimeout(() => setCopiedId(null), 1600);
    }
  };

  const renderJobRow = (job: Job) => {
    const isSelected = selectedIds.includes(job.id);
    const isExpanded = expandedId === job.id;
    const primaryOut = job.outputs[0] ?? null;
    const isActive =
      job.state === "downloading" ||
      job.state === "postprocessing" ||
      job.state === "workshop" ||
      job.state === "probing" ||
      job.state === "waiting_premiere";

    return (
      <ContextMenu.Root key={job.id}>
        <ContextMenu.Trigger asChild>
          <motion.div
            layout
            data-job-row-id={job.id}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.98 }}
            transition={SPRING_UI}
            draggable={job.state === "queued" || job.state === "paused"}
            onDragStart={() => setDraggingId(job.id)}
            onDragOver={(e) => {
              if (draggingId) e.preventDefault();
            }}
            onDrop={() => handleDropReorder(job.id)}
            onClick={(e) => toggleSelect(job.id, e.ctrlKey || e.metaKey)}
            className="op-panel"
            style={{
              padding: "10px 14px",
              borderColor:
                job.state === "failed"
                  ? "var(--warn)"
                  : isSelected
                  ? "var(--text)"
                  : "var(--line)",
              display: "flex",
              flexDirection: "column",
              gap: 8,
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
              {/* Reorder / Native File Drag-out Handle */}
              <div
                title={
                  job.state === "done" && primaryOut
                    ? "Arraste para fora (Explorer, Premiere, DAW)"
                    : "Arraste para reordenar na fila"
                }
                onMouseDown={(e) => {
                  if (job.state === "done" && primaryOut) {
                    e.stopPropagation();
                    api.startFileDrag(primaryOut);
                  }
                }}
                style={{
                  color: "var(--text-3)",
                  cursor: job.state === "done" && primaryOut ? "grab" : "ns-resize",
                  display: "flex",
                  alignItems: "center",
                }}
              >
                <GripVertical size={15} />
              </div>

              {/* Thumbnail */}
              <div
                style={{
                  width: 56,
                  height: 36,
                  borderRadius: 6,
                  overflow: "hidden",
                  background: "var(--surface-2)",
                  border: "1px solid var(--line)",
                  flexShrink: 0,
                }}
              >
                {job.thumb_url && (
                  <img
                    src={job.thumb_url}
                    alt=""
                    style={{ width: "100%", height: "100%", objectFit: "cover" }}
                  />
                )}
              </div>

              {/* Title + Platform + Progress */}
              <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 5 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  {job.state === "done" && <AnimatedDoneCheck />}
                  <PlatformIcon
                    slug={job.source.icon_slug}
                    color={job.source.brand_color}
                    size={14}
                  />
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
                    {job.playlist_index ? `${String(job.playlist_index).padStart(2, "0")} - ` : ""}
                    {job.title}
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
                      flexShrink: 0,
                    }}
                  >
                    {formatJobBadge(job)}
                  </span>
                </div>

                <StripeProgress
                  pct={job.progress.pct}
                  state={job.state}
                  speedBps={job.progress.speed_bps}
                  height={8}
                />

                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    fontSize: 12,
                    color: job.state === "failed" ? "var(--warn)" : "var(--text-2)",
                  }}
                >
                  <span>
                    {job.state === "failed"
                      ? job.error?.message_pt || "Falha no download"
                      : job.progress.stage}
                  </span>
                  <div
                    className="font-mono-num"
                    style={{ display: "flex", gap: 12, whiteSpace: "nowrap", flexShrink: 0 }}
                  >
                    {isActive && job.progress.speed_bps > 0 && (
                      <span>{formatSpeedPrimary(job.progress.speed_bps, settings.speed_unit)}</span>
                    )}
                    {isActive && job.progress.eta_s !== null && (
                      <span>ETA {formatEta(job.progress.eta_s)}</span>
                    )}
                    <span>
                      {job.progress.approximate ? "~" : ""}
                      {formatBytes(job.progress.total_bytes ?? job.estimated_bytes ?? job.progress.downloaded_bytes)}
                    </span>
                    <span>{Math.round(job.progress.pct * 100)}%</span>
                  </div>
                </div>
              </div>

              {/* Row Actions */}
              <div
                onClick={(e) => e.stopPropagation()}
                style={{ display: "flex", alignItems: "center", gap: 4, flexShrink: 0 }}
              >
                {(isActive || job.state === "queued") && (
                  <Button size="sm" variant="secondary" onClick={() => api.pause(job.id)} title="Pausar">
                    <Pause size={13} />
                  </Button>
                )}
                {job.state === "paused" && (
                  <Button size="sm" variant="secondary" onClick={() => api.resume(job.id)} title="Retomar">
                    <Play size={13} />
                  </Button>
                )}
                {job.state === "failed" && (
                  <Button size="sm" variant="warn" onClick={() => api.retry(job.id)} title="Tentar de novo">
                    <RotateCcw size={13} />
                  </Button>
                )}
                {job.state === "done" && primaryOut && (
                  <>
                    <Button size="sm" variant="secondary" onClick={() => api.openFile(primaryOut)} title="Abrir arquivo">
                      <ExternalLink size={13} />
                    </Button>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => api.revealInExplorer(primaryOut)}
                      title="Mostrar na pasta"
                    >
                      <FolderOpen size={13} />
                    </Button>
                  </>
                )}
                {!["done", "canceled"].includes(job.state) && (
                  <Button size="sm" variant="ghost" onClick={() => api.cancel(job.id)} title="Cancelar">
                    <X size={14} />
                  </Button>
                )}
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setExpandedId(job.id)}
                  title="Detalhes e log"
                >
                  {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                </Button>
              </div>
            </div>

            {/* Expandable Details, Error Action & 300-line Monospace Log */}
            {isExpanded && (
              <div
                onClick={(e) => e.stopPropagation()}
                style={{
                  marginTop: 4,
                  paddingTop: 10,
                  borderTop: "1px solid var(--line)",
                  display: "flex",
                  flexDirection: "column",
                  gap: 10,
                }}
              >
                {job.error && (
                  <div
                    style={{
                      padding: "10px 12px",
                      borderRadius: 8,
                      border: "1px solid var(--warn)",
                      background: "var(--surface-2)",
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      gap: 12,
                    }}
                  >
                    <div style={{ display: "flex", gap: 10, alignItems: "flex-start" }}>
                      <AlertTriangle size={16} color="var(--warn)" style={{ marginTop: 2 }} />
                      <div>
                        <div style={{ fontWeight: 700, color: "var(--warn)", fontSize: 13 }}>
                          {job.error.message_pt}
                        </div>
                        {job.error.hint_pt && (
                          <div style={{ fontSize: 12, color: "var(--text-2)" }}>
                            {job.error.hint_pt}
                          </div>
                        )}
                      </div>
                    </div>
                    <div style={{ display: "flex", gap: 6 }}>
                      {job.error.action.type === "login" && (
                        <Button
                          size="sm"
                          variant="warn"
                          onClick={() => setActiveTab("settings", "accounts")}
                        >
                          Conectar conta
                        </Button>
                      )}
                      {job.error.action.type === "update_ytdlp" && (
                        <Button
                          size="sm"
                          variant="warn"
                          onClick={async () => {
                            await api.updateYtdlp();
                            await api.retry(job.id);
                          }}
                        >
                          Atualizar yt-dlp e tentar de novo
                        </Button>
                      )}
                      {(job.error.action.type === "change_format" ||
                        job.error.action.type === "change_destination") && (
                        <Button
                          size="sm"
                          variant="warn"
                          onClick={() => {
                            setPendingUrlToLoad(job.url);
                            setActiveTab("download");
                          }}
                        >
                          Ajustar opções
                        </Button>
                      )}
                      <Button size="sm" variant="secondary" onClick={() => api.retry(job.id)}>
                        Tentar de novo
                      </Button>
                    </div>
                  </div>
                )}

                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8 }}>
                  <span className="font-mono-num" style={{ fontSize: 11, color: "var(--text-2)" }}>
                    Destino: {job.outputs[0] || job.spec.output_root}
                  </span>
                  <div style={{ display: "flex", gap: 6 }}>
                    <Button size="sm" variant="secondary" onClick={() => copyCommand(job)}>
                      <Copy size={12} />{" "}
                      {copiedId === job.id ? "Copiado!" : "Copiar comando equivalente"}
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => api.removeJobs([job.id])}>
                      <Trash2 size={12} /> Remover da lista
                    </Button>
                  </div>
                </div>

                {job.log_tail.length > 0 && (
                  <pre
                    className="font-mono-num selectable"
                    style={{
                      margin: 0,
                      padding: 10,
                      maxHeight: 160,
                      overflowY: "auto",
                      background: "var(--bg)",
                      border: "1px solid var(--line)",
                      borderRadius: 6,
                      fontSize: 11,
                      color: "var(--text-2)",
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-all",
                    }}
                  >
                    {job.log_tail.join("\n")}
                  </pre>
                )}
              </div>
            )}
          </motion.div>
        </ContextMenu.Trigger>

        <ContextMenu.Portal>
          <ContextMenu.Content
            className="op-panel"
            style={{ padding: 6, minWidth: 200, zIndex: 80 }}
          >
            {[
              primaryOut && {
                label: "Abrir arquivo",
                onSelect: () => api.openFile(primaryOut),
              },
              primaryOut && {
                label: "Mostrar na pasta",
                onSelect: () => api.revealInExplorer(primaryOut),
              },
              {
                label: "Copiar comando equivalente",
                onSelect: () => copyCommand(job),
              },
              {
                label: "Remover da fila",
                onSelect: () =>
                  api.removeJobs(selectedIds.includes(job.id) ? selectedIds : [job.id]),
              },
            ]
              .filter(Boolean)
              .map((item: any) => (
                <ContextMenu.Item
                  key={item.label}
                  onSelect={item.onSelect}
                  style={{
                    padding: "7px 10px",
                    fontSize: 12.5,
                    borderRadius: 6,
                    cursor: "pointer",
                    color: "var(--text)",
                    outline: "none",
                  }}
                >
                  {item.label}
                </ContextMenu.Item>
              ))}
          </ContextMenu.Content>
        </ContextMenu.Portal>
      </ContextMenu.Root>
    );
  };

  const renderGroupBlock = (title: string, items: Job[]) => {
    if (items.length === 0) return null;
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <div
          className="font-display"
          style={{ fontSize: 12, color: "var(--text-2)", marginTop: 6 }}
        >
          {title} ({items.length})
        </div>
        <AnimatePresence initial={false}>{items.map(renderJobRow)}</AnimatePresence>
      </div>
    );
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
      <div style={{ maxWidth: 940, margin: "0 auto", display: "flex", flexDirection: "column", gap: 14 }}>
        {/* Queue Toolbar */}
        <div
          className="op-panel"
          style={{
            padding: "10px 14px",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            flexWrap: "wrap",
            gap: 12,
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span style={{ fontSize: 13, color: "var(--text-2)" }}>Downloads simultâneos:</span>
            <Button size="sm" variant="secondary" onClick={() => changeConcurrency(-1)}>
              <Minus size={12} />
            </Button>
            <span className="font-mono-num" style={{ fontWeight: 700, width: 18, textAlign: "center" }}>
              {settings.global_concurrency}
            </span>
            <Button size="sm" variant="secondary" onClick={() => changeConcurrency(1)}>
              <Plus size={12} />
            </Button>
          </div>

          <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            <Button size="sm" variant="secondary" onClick={() => api.pauseAll()}>
              <Pause size={13} /> Pausar tudo
            </Button>
            <Button size="sm" variant="secondary" onClick={() => api.resumeAll()}>
              <Play size={13} /> Retomar tudo
            </Button>
            <Button size="sm" variant="ghost" onClick={() => api.clearCompleted()}>
              <Trash2 size={13} /> Limpar concluídos
            </Button>
          </div>
        </div>

        {/* Playlist Group Summary Headers */}
        {Array.from(playlistGroups.entries()).map(([gid, g]) => (
          <div
            key={gid}
            className="op-panel"
            style={{
              padding: "10px 14px",
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: 14,
            }}
          >
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 13, fontWeight: 700 }}>
                Playlist: {g.title} ({g.done}/{g.total} concluídos)
              </div>
              <div style={{ marginTop: 6 }}>
                <StripeProgress
                  pct={g.total > 0 ? g.pctSum / g.total : 0}
                  state={g.done === g.total ? "done" : "downloading"}
                  speedBps={0}
                  height={6}
                />
              </div>
            </div>
            <div style={{ display: "flex", gap: 6 }}>
              <Button size="sm" variant="secondary" onClick={() => api.pauseGroup(gid)}>
                Pausar grupo
              </Button>
              <Button size="sm" variant="secondary" onClick={() => api.resumeGroup(gid)}>
                Retomar grupo
              </Button>
            </div>
          </div>
        ))}

        {/* Empty State (Section 14.6 exact microcopy) */}
        {jobs.length === 0 ? (
          <div
            className="op-panel"
            style={{
              padding: "56px 24px",
              textAlign: "center",
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              gap: 12,
            }}
          >
            <div style={{ fontSize: 16, color: "var(--text-2)" }}>
              Nada na fila. Cola um link lá em cima.
            </div>
            <Button variant="primary" onClick={() => setActiveTab("download")}>
              Ir para Baixar
            </Button>
          </div>
        ) : (
          <>
            {renderGroupBlock("Ativos", sections.active)}
            {renderGroupBlock("Falhas", sections.failed)}
            {renderGroupBlock("Esperando", sections.waiting)}
            {renderGroupBlock("Concluídos hoje", sections.doneToday)}
          </>
        )}
      </div>
    </div>
  );
};
