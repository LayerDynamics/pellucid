import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";

export interface PanelLayout {
  id: string;
  rowSpan: number;
  colSpan: number;
  hidden: boolean;
  order: number;
}

export interface PanelState {
  layouts: Record<string, PanelLayout>;
  hidden: Set<string>;
  highlightedPanelId: string | null;
  setLayout: (panelId: string, layout: Partial<PanelLayout>) => void;
  hide: (panelId: string) => void;
  show: (panelId: string) => void;
  reset: () => void;
  isHidden: (panelId: string) => boolean;
  getLayout: (panelId: string) => PanelLayout | undefined;
  registered: () => string[];
  setHighlighted: (panelId: string | null) => void;
}

const defaultLayout = (id: string, order: number): PanelLayout => ({
  id,
  rowSpan: 1,
  colSpan: 1,
  hidden: false,
  order,
});

export const usePanelStore = create<PanelState>()(
  subscribeWithSelector((set, get) => ({
    layouts: {},
    hidden: new Set<string>(),
    highlightedPanelId: null,
    setLayout: (panelId, layout) =>
      set((s) => {
        const existing = s.layouts[panelId] ?? defaultLayout(panelId, Object.keys(s.layouts).length);
        const merged: PanelLayout = { ...existing, ...layout, id: panelId };
        return { layouts: { ...s.layouts, [panelId]: merged } };
      }),
    hide: (panelId) =>
      set((s) => {
        const next = new Set(s.hidden);
        next.add(panelId);
        return { hidden: next };
      }),
    show: (panelId) =>
      set((s) => {
        const next = new Set(s.hidden);
        next.delete(panelId);
        return { hidden: next };
      }),
    reset: () => set({ layouts: {}, hidden: new Set(), highlightedPanelId: null }),
    isHidden: (panelId) => get().hidden.has(panelId),
    getLayout: (panelId) => get().layouts[panelId],
    registered: () => Object.keys(get().layouts),
    setHighlighted: (panelId) => set({ highlightedPanelId: panelId }),
  })),
);
