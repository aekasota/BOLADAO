import { create } from "zustand";
import { BandwidthProfile, GameModeStatus, LimiterStatsPayload } from "../lib/ipc";

interface LimiterState {
  stats: LimiterStatsPayload;
  gameMode: GameModeStatus;
  setStats: (stats: LimiterStatsPayload) => void;
  setGameMode: (gameMode: GameModeStatus) => void;
}

export const useLimiterStore = create<LimiterState>((set) => ({
  stats: {
    speed_bps: 0,
    limit_bps: 0,
    profile: "unlimited" as BandwidthProfile,
    peak_7d_bps: 0,
    sparkline_60s: new Array(60).fill(0),
    proxy_port: null,
    fallback_mode: false,
    game_mode_override_active: false,
  },
  gameMode: {
    active: false,
    override_mode: "auto",
    reason: null,
  },
  setStats: (stats) => set({ stats }),
  setGameMode: (gameMode) => set({ gameMode }),
}));
