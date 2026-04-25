import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";

export type CorrelationDomain = "military" | "escalation" | "economic" | "disaster";

export interface CorrelationResult {
  id: string;
  domain: CorrelationDomain;
  score: number;
  fingerprint: string;
  computedAtMs: number;
}

export interface CorrelationState {
  active: CorrelationDomain[];
  results: Record<string, CorrelationResult>;
  lastRunAtMs: number | null;
  setActive: (domains: CorrelationDomain[]) => void;
  toggleDomain: (domain: CorrelationDomain) => void;
  setResult: (result: CorrelationResult) => void;
  clearResults: () => void;
  resultsForDomain: (domain: CorrelationDomain) => CorrelationResult[];
}

export const useCorrelationStore = create<CorrelationState>()(
  subscribeWithSelector((set, get) => ({
    active: ["military", "escalation", "economic", "disaster"],
    results: {},
    lastRunAtMs: null,
    setActive: (domains) => set({ active: [...new Set(domains)] }),
    toggleDomain: (domain) =>
      set((s) => ({
        active: s.active.includes(domain)
          ? s.active.filter((d) => d !== domain)
          : [...s.active, domain],
      })),
    setResult: (result) =>
      set((s) => ({
        results: { ...s.results, [result.id]: result },
        lastRunAtMs: result.computedAtMs,
      })),
    clearResults: () => set({ results: {}, lastRunAtMs: null }),
    resultsForDomain: (domain) =>
      Object.values(get().results).filter((r) => r.domain === domain),
  })),
);
