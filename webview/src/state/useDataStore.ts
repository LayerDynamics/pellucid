import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";

export interface PanelData<T = unknown> {
  data: T;
  fetchedAtMs: number;
  isStale: boolean;
}

export interface DataState {
  byPanel: Record<string, PanelData>;
  lastFetchedAtMs: Record<string, number>;
  set: <T>(panelId: string, payload: T, opts?: { isStale?: boolean }) => void;
  markStale: (panelId: string) => void;
  clear: (panelId: string) => void;
  get: <T = unknown>(panelId: string) => PanelData<T> | undefined;
  size: () => number;
}

export const useDataStore = create<DataState>()(
  subscribeWithSelector((set, get) => ({
    byPanel: {},
    lastFetchedAtMs: {},
    set: (panelId, payload, opts) => {
      const now = Date.now();
      set((s) => ({
        byPanel: {
          ...s.byPanel,
          [panelId]: { data: payload, fetchedAtMs: now, isStale: opts?.isStale ?? false },
        },
        lastFetchedAtMs: { ...s.lastFetchedAtMs, [panelId]: now },
      }));
    },
    markStale: (panelId) =>
      set((s) => {
        const cur = s.byPanel[panelId];
        if (!cur) return s;
        return { byPanel: { ...s.byPanel, [panelId]: { ...cur, isStale: true } } };
      }),
    clear: (panelId) =>
      set((s) => {
        const next = { ...s.byPanel };
        delete next[panelId];
        const ts = { ...s.lastFetchedAtMs };
        delete ts[panelId];
        return { byPanel: next, lastFetchedAtMs: ts };
      }),
    get: <T = unknown>(panelId: string) => get().byPanel[panelId] as PanelData<T> | undefined,
    size: () => Object.keys(get().byPanel).length,
  })),
);
