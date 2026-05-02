import { create } from 'zustand'
import type { MaterialDefinition } from '@/lib/plastiq-engine/Material/MaterialSchema'

// SPEC-1 M-01..M-14, §6.2 — Material Creator Studio store.
//
// State for the studio surface: which inspector tab is open, the layer
// stack the LayerCompositor consumes, the preview-geometry choice,
// preview environment, and the working material definition.
//
// The actual layer compilation pipeline (M-10) lives in the studio's
// engine module; this store is the source-of-truth for editor state.
// Layers carry their own PBR-channel scope so the inspector tab can
// show only the layers relevant to the active channel.

export type InspectorTab =
  | 'base'
  | 'clearcoat'
  | 'sheen'
  | 'transmission'
  | 'iridescence'
  | 'anisotropy'
  | 'specular'
  | 'emission'
  | 'displacement'
  | 'opacity'
  | 'environment'
  | 'procedural'
  | 'layers'

export type PreviewGeometry =
  | 'sphere'
  | 'cube'
  | 'cylinder'
  | 'torus'
  | 'bunny'
  | 'loaded-mesh'

export type LayerBlendMode =
  | 'normal'
  | 'multiply'
  | 'screen'
  | 'overlay'
  | 'add'
  | 'subtract'
  | 'lighten'
  | 'darken'

export type LayerScope =
  | 'base'
  | 'clearcoat'
  | 'sheen'
  | 'transmission'
  | 'iridescence'
  | 'anisotropy'
  | 'specular'
  | 'emission'
  | 'displacement'
  | 'opacity'
  | 'normal'

export interface MaterialLayer {
  id: string
  name: string
  scope: LayerScope
  enabled: boolean
  blendMode: LayerBlendMode
  /** 0..1 layer opacity. */
  opacity: number
  /** Optional layer-mask texture (URL or blob: URL). */
  maskUrl: string | null
  /** Optional adjustment-layer parameters (hue/sat/value/contrast). */
  adjustment: {
    hue: number
    saturation: number
    value: number
    contrast: number
  } | null
  /** Free-form group / label text — used by group + label-layer rows. */
  label: string | null
  /** Hierarchical parent — null for top-level layers. */
  parentId: string | null
  /** Layer parameters keyed by name; LayerCompositor consumes these. */
  params: Record<string, unknown>
}

interface MaterialStudioState {
  open: boolean
  inspectorTab: InspectorTab
  previewGeometry: PreviewGeometry
  hdriPresetId: string

  /** The currently-edited material definition, in MaterialSchema shape. */
  definition: MaterialDefinition | null

  /** Layer stack (bottom → top). */
  layers: MaterialLayer[]
  /** Currently selected layer id (drives the inspector). */
  activeLayerId: string | null

  /** Whether the working state has unsaved changes vs the loaded preset. */
  dirty: boolean

  setOpen(open: boolean): void
  setInspectorTab(tab: InspectorTab): void
  setPreviewGeometry(geom: PreviewGeometry): void
  setHdriPreset(id: string): void

  loadDefinition(def: MaterialDefinition): void
  patchDefinition(patch: Partial<MaterialDefinition>): void

  addLayer(layer: MaterialLayer): void
  updateLayer(id: string, patch: Partial<Omit<MaterialLayer, 'id'>>): void
  removeLayer(id: string): void
  reorderLayers(fromIndex: number, toIndex: number): boolean
  setActiveLayer(id: string | null): void

  markClean(): void
  reset(): void
}

const defaults: Pick<
  MaterialStudioState,
  | 'open'
  | 'inspectorTab'
  | 'previewGeometry'
  | 'hdriPresetId'
  | 'definition'
  | 'layers'
  | 'activeLayerId'
  | 'dirty'
> = {
  open: false,
  inspectorTab: 'base',
  previewGeometry: 'sphere',
  hdriPresetId: 'studio',
  definition: null,
  layers: [],
  activeLayerId: null,
  dirty: false,
}

export const useMaterialStudioStore = create<MaterialStudioState>((set) => ({
  ...defaults,

  setOpen(open) {
    set({ open })
  },

  setInspectorTab(inspectorTab) {
    set({ inspectorTab })
  },

  setPreviewGeometry(previewGeometry) {
    set({ previewGeometry })
  },

  setHdriPreset(id) {
    set({ hdriPresetId: id, dirty: true })
  },

  loadDefinition(def) {
    set({ definition: def, dirty: false })
  },

  patchDefinition(patch) {
    set((state) => ({
      definition: state.definition ? { ...state.definition, ...patch } : (patch as MaterialDefinition),
      dirty: true,
    }))
  },

  addLayer(layer) {
    set((state) => ({
      layers: [...state.layers, layer],
      activeLayerId: layer.id,
      dirty: true,
    }))
  },

  updateLayer(id, patch) {
    set((state) => ({
      layers: state.layers.map((l) => (l.id === id ? { ...l, ...patch } : l)),
      dirty: true,
    }))
  },

  removeLayer(id) {
    set((state) => {
      const next = state.layers.filter((l) => l.id !== id)
      return {
        layers: next,
        activeLayerId: state.activeLayerId === id ? null : state.activeLayerId,
        dirty: true,
      }
    })
  },

  reorderLayers(fromIndex, toIndex) {
    let mutated = false
    set((state) => {
      if (
        fromIndex < 0 ||
        toIndex < 0 ||
        fromIndex >= state.layers.length ||
        toIndex >= state.layers.length ||
        fromIndex === toIndex
      ) {
        return state
      }
      mutated = true
      const next = state.layers.slice()
      const [item] = next.splice(fromIndex, 1)
      next.splice(toIndex, 0, item)
      return { layers: next, dirty: true }
    })
    return mutated
  },

  setActiveLayer(activeLayerId) {
    set({ activeLayerId })
  },

  markClean() {
    set({ dirty: false })
  },

  reset() {
    set(defaults)
  },
}))
