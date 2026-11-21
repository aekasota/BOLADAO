import { create } from "zustand";
import { api, Job, JobState, Progress } from "../lib/ipc";

interface QueueState {
  jobs: Job[];
  selectedIds: string[];
  expandedId: string | null;
  loadQueue: () => Promise<void>;
  setJobs: (jobs: Job[]) => void;
  applyProgressEvent: (ev: {
    id: string;
    state: JobState;
    progress: Progress;
    equivalent_cmd: string;
  }) => void;
  toggleSelect: (id: string, multi: boolean) => void;
  clearSelection: () => void;
  setExpandedId: (id: string | null) => void;
}

export const useQueueStore = create<QueueState>((set) => ({
  jobs: [],
  selectedIds: [],
  expandedId: null,

  loadQueue: async () => {
    const jobs = await api.getQueue();
    set({ jobs });
  },

  setJobs: (jobs) => set({ jobs }),

  applyProgressEvent: (ev) =>
    set((st) => ({
      jobs: st.jobs.map((j) =>
        j.id === ev.id
          ? {
              ...j,
              state: ev.state,
              progress: ev.progress,
              equivalent_cmd: ev.equivalent_cmd || j.equivalent_cmd,
            }
          : j
      ),
    })),

  toggleSelect: (id, multi) =>
    set((st) => {
      if (!multi) {
        return {
          selectedIds: st.selectedIds.length === 1 && st.selectedIds[0] === id ? [] : [id],
        };
      }
      return {
        selectedIds: st.selectedIds.includes(id)
          ? st.selectedIds.filter((x) => x !== id)
          : [...st.selectedIds, id],
      };
    }),

  clearSelection: () => set({ selectedIds: [] }),

  setExpandedId: (id) =>
    set((st) => ({ expandedId: st.expandedId === id ? null : id })),
}));
