import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";

export type MapMode = "2d" | "3d";

export interface Viewport {
  longitude: number;
  latitude: number;
  zoom: number;
  bearing: number;
  pitch: number;
}

export interface MapState {
  mode: MapMode;
  viewport: Viewport;
  layers: string[];
  selectedFeatureId: string | null;
  setMode: (mode: MapMode) => void;
  setViewport: (viewport: Partial<Viewport>) => void;
  setLayers: (layers: string[]) => void;
  toggleLayer: (id: string) => void;
  selectFeature: (id: string | null) => void;
  resetLayers: () => void;
}

const DEFAULT_VIEWPORT: Viewport = {
  longitude: 0,
  latitude: 20,
  zoom: 2,
  bearing: 0,
  pitch: 0,
};

export const useMapStore = create<MapState>()(
  subscribeWithSelector((set, get) => ({
    mode: "2d",
    viewport: DEFAULT_VIEWPORT,
    layers: [],
    selectedFeatureId: null,
    setMode: (mode) => set({ mode }),
    setViewport: (vp) => set((s) => ({ viewport: { ...s.viewport, ...vp } })),
    setLayers: (layers) => set({ layers: [...layers] }),
    toggleLayer: (id) =>
      set((s) => ({
        layers: s.layers.includes(id) ? s.layers.filter((x) => x !== id) : [...s.layers, id],
      })),
    selectFeature: (id) => set({ selectedFeatureId: id }),
    resetLayers: () => set({ layers: [], selectedFeatureId: null }),
  })),
);
