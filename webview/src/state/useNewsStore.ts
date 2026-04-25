import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";

export interface NewsItem {
  id: string;
  title: string;
  source: string;
  publishedAtMs: number;
  url?: string;
  summary?: string;
  severity?: "info" | "warn" | "high" | "critical";
}

export interface NewsState {
  feed: NewsItem[];
  breaking: NewsItem | null;
  signals: string[];
  gaps: string[];
  setFeed: (items: NewsItem[]) => void;
  prepend: (item: NewsItem) => void;
  setBreaking: (item: NewsItem | null) => void;
  setSignals: (signals: string[]) => void;
  setGaps: (gaps: string[]) => void;
  clear: () => void;
  size: () => number;
}

export const useNewsStore = create<NewsState>()(
  subscribeWithSelector((set, get) => ({
    feed: [],
    breaking: null,
    signals: [],
    gaps: [],
    setFeed: (items) => set({ feed: [...items] }),
    prepend: (item) =>
      set((s) => ({ feed: [item, ...s.feed.filter((x) => x.id !== item.id)] })),
    setBreaking: (item) => set({ breaking: item }),
    setSignals: (signals) => set({ signals: [...signals] }),
    setGaps: (gaps) => set({ gaps: [...gaps] }),
    clear: () => set({ feed: [], breaking: null, signals: [], gaps: [] }),
    size: () => get().feed.length,
  })),
);
