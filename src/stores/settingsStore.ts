import { create } from "zustand";
import { api, AppSettings, fallbackSettings, Preset } from "../lib/ipc";

interface SettingsState {
  settings: AppSettings;
  presets: Preset[];
  loaded: boolean;
  loadSettings: () => Promise<void>;
  updateSettings: (patch: Partial<AppSettings>) => Promise<void>;
  refreshPresets: () => Promise<void>;
}

function applyDomTheme(s: AppSettings) {
  if (typeof document === "undefined") return;
  document.documentElement.setAttribute("data-theme", s.theme);
  document.documentElement.setAttribute(
    "data-reduce-motion",
    s.reduce_motion ? "true" : "false"
  );
}

export const useSettingsStore = create<SettingsState>((set, get) => ({
  settings: fallbackSettings(),
  presets: [],
  loaded: false,

  loadSettings: async () => {
    const [settings, presets] = await Promise.all([api.getSettings(), api.getPresets()]);
    applyDomTheme(settings);
    set({ settings, presets, loaded: true });
  },

  updateSettings: async (patch) => {
    const next: AppSettings = { ...get().settings, ...patch };
    applyDomTheme(next);
    set({ settings: next });
    await api.saveSettings(next);
  },

  refreshPresets: async () => {
    const presets = await api.getPresets();
    set({ presets });
  },
}));
