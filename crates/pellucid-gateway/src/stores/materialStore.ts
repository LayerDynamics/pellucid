import { create } from 'zustand'
import type { MaterialDefinition } from '@/lib/plastiq-engine/Material/MaterialSchema'

export interface MaterialLayer {
  id: string
  name: string
  type: 'base' | 'roughness' | 'metalness' | 'normal' | 'emissive' | 'opacity'
  enabled: boolean
  value: number
  color?: string
  textureUrl?: string | null
}

interface MaterialState {
  layers: MaterialLayer[]
  activeLayerId: string | null
  previewAssetId: string | null
  /**
   * SPEC-2 §6 — `materialStore.activeDefinition` is the current
   * MaterialDefinition broadcast through `Viewport/Material` and
   * `Object/Material` to every scene mesh. Null means "use the per-mesh
   * material" (no override active).
   */
  activeDefinition: MaterialDefinition | null
  /** Identifier of the active library preset (`'plastic'`, `'toon'`, or a KMP id). */
  activePresetId: string | null

  setLayers: (layers: MaterialLayer[]) => void
  addLayer: (layer: MaterialLayer) => void
  updateLayer: (id: string, patch: Partial<MaterialLayer>) => void
  removeLayer: (id: string) => void
  setActiveLayer: (id: string | null) => void
  setPreviewAsset: (id: string | null) => void
  setActiveDefinition: (def: MaterialDefinition | null) => void
  setActivePreset: (id: string | null, def: MaterialDefinition | null) => void
  reset: () => void
}

const defaultLayers: MaterialLayer[] = [
  { id: 'base', name: 'Base Color', type: 'base', enabled: true, value: 1, color: '#cccccc' },
  { id: 'roughness', name: 'Roughness', type: 'roughness', enabled: true, value: 0.5 },
  { id: 'metalness', name: 'Metalness', type: 'metalness', enabled: true, value: 0 },
]

export const useMaterialStore = create<MaterialState>((set) => ({
  layers: defaultLayers,
  activeLayerId: 'base',
  previewAssetId: null,
  activeDefinition: null,
  activePresetId: null,

  setLayers: (layers) => set({ layers }),
  addLayer: (layer) => set((state) => ({ layers: [...state.layers, layer] })),
  updateLayer: (id, patch) =>
    set((state) => ({
      layers: state.layers.map((l) => (l.id === id ? { ...l, ...patch } : l)),
    })),
  removeLayer: (id) =>
    set((state) => ({ layers: state.layers.filter((l) => l.id !== id) })),
  setActiveLayer: (activeLayerId) => set({ activeLayerId }),
  setPreviewAsset: (previewAssetId) => set({ previewAssetId }),
  setActiveDefinition: (activeDefinition) => set({ activeDefinition }),
  setActivePreset: (activePresetId, activeDefinition) =>
    set({ activePresetId, activeDefinition }),
  reset: () =>
    set({
      layers: defaultLayers,
      activeLayerId: 'base',
      previewAssetId: null,
      activeDefinition: null,
      activePresetId: null,
    }),
}))
