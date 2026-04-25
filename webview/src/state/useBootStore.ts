import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";

/**
 * 8-phase boot machine per SPEC-001 §3.3 Path A. Each phase advances
 * once its predecessor's awaitable resolves; an error transitions the
 * whole machine to "errored" without short-circuiting subsequent phases'
 * cleanup hooks (handled in T1.11).
 */
export type BootPhase =
  | "idle"
  | "p1-storage-i18n-ml-init"
  | "p2-bootstrap-fast-slow"
  | "p3-clerk-auth"
  | "p4-panel-layout"
  | "p5-search-intel-url-state"
  | "p6-parallel-data-load"
  | "p7-smart-poll-loop"
  | "p8-desktop-updater"
  | "ready"
  | "errored";

const PHASE_ORDER: BootPhase[] = [
  "idle",
  "p1-storage-i18n-ml-init",
  "p2-bootstrap-fast-slow",
  "p3-clerk-auth",
  "p4-panel-layout",
  "p5-search-intel-url-state",
  "p6-parallel-data-load",
  "p7-smart-poll-loop",
  "p8-desktop-updater",
  "ready",
];

export interface BootState {
  phase: BootPhase;
  startedAtMs: number | null;
  reachedAtMs: Partial<Record<BootPhase, number>>;
  errorMessage: string | null;
  start: () => void;
  advance: (next: BootPhase) => void;
  fail: (message: string) => void;
  reset: () => void;
  isReady: () => boolean;
  hasReached: (phase: BootPhase) => boolean;
}

const phaseRank = (phase: BootPhase): number => {
  const idx = PHASE_ORDER.indexOf(phase);
  return idx === -1 ? -1 : idx;
};

export const useBootStore = create<BootState>()(
  subscribeWithSelector((set, get) => ({
    phase: "idle",
    startedAtMs: null,
    reachedAtMs: {},
    errorMessage: null,
    start: () =>
      set({
        phase: "p1-storage-i18n-ml-init",
        startedAtMs: Date.now(),
        reachedAtMs: { idle: Date.now() },
        errorMessage: null,
      }),
    advance: (next) => {
      // Refuse to "advance" backwards or skip phases.
      const current = get().phase;
      if (current === "errored") return;
      const cur = phaseRank(current);
      const nxt = phaseRank(next);
      if (nxt === -1 || nxt <= cur) return;
      set((s) => ({
        phase: next,
        reachedAtMs: { ...s.reachedAtMs, [next]: Date.now() },
      }));
    },
    fail: (message) => set({ phase: "errored", errorMessage: message }),
    reset: () =>
      set({ phase: "idle", startedAtMs: null, reachedAtMs: {}, errorMessage: null }),
    isReady: () => get().phase === "ready",
    hasReached: (phase) => phaseRank(get().phase) >= phaseRank(phase),
  })),
);
