import { create } from "zustand";
import { BinsStatusPayload } from "../lib/ipc";

export type AppTab = "download" | "queue" | "library" | "settings";

export interface StageRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface RippleEvent {
  id: number;
  x: number;
  y: number;
  strength: number;
}

interface AppUiState {
  activeTab: AppTab;
  settingsSection: string;
  stageRect: StageRect;
  urlTargetCenter: { x: number; y: number };
  dragOverActive: boolean;
  rippleEvent: RippleEvent | null;
  clipboardSuggestion: { url: string; platformName: string } | null;
  pendingUrlToLoad: string | null;
  binsStatus: BinsStatusPayload;
  setActiveTab: (tab: AppTab, settingsSection?: string) => void;
  setStageRect: (rect: StageRect) => void;
  setUrlTargetCenter: (x: number, y: number) => void;
  setDragOverActive: (active: boolean) => void;
  fireRipple: (x: number, y: number, strength?: number) => void;
  setClipboardSuggestion: (sug: { url: string; platformName: string } | null) => void;
  setPendingUrlToLoad: (url: string | null) => void;
  setBinsStatus: (st: BinsStatusPayload) => void;
}

export const useAppStore = create<AppUiState>((set) => ({
  activeTab: "download",
  settingsSection: "folders",
  stageRect: { x: 0, y: 44, width: 1120, height: 260 },
  urlTargetCenter: { x: 560, y: 150 },
  dragOverActive: false,
  rippleEvent: null,
  clipboardSuggestion: null,
  pendingUrlToLoad: null,
  binsStatus: {
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
      hw_encoders: [],
      sw_encoders: [],
    },
    error: null,
  },
  setActiveTab: (tab, settingsSection) =>
    set((prev) => ({
      activeTab: tab,
      settingsSection: settingsSection ?? prev.settingsSection,
    })),
  setStageRect: (stageRect) => set({ stageRect }),
  setUrlTargetCenter: (x, y) => set({ urlTargetCenter: { x, y } }),
  setDragOverActive: (dragOverActive) => set({ dragOverActive }),
  fireRipple: (x, y, strength = 1.0) =>
    set({ rippleEvent: { id: Date.now() + Math.random(), x, y, strength } }),
  setClipboardSuggestion: (clipboardSuggestion) => set({ clipboardSuggestion }),
  setPendingUrlToLoad: (pendingUrlToLoad) => set({ pendingUrlToLoad }),
  setBinsStatus: (binsStatus) => set({ binsStatus }),
}));
