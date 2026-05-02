import { create } from 'zustand'
import type * as THREE from 'three'
import type { AssetSummary } from '@/lib/plastiq-client'
import type {
  Selection,
  SelectionMode,
} from '@/lib/plastiq-engine/Select/Select'
import { selectionsEqual } from '@/lib/plastiq-engine/Select/Select'

// ── SceneObjectEntry ────────────────────────────────────────────────────────
//
// SPEC-2 §6 — `contextStore` tracks the loaded scene objects so DOM
// controls (`ObjectControls`, `ObjectList`) can drive selection,
// visibility, and per-object transforms without prop-drilling. Each
// entry pairs a stable id with the live THREE.Object3D so consumers
// can mutate the scene graph directly.

export interface SceneObjectEntry {
  id: string
  name: string
  object: THREE.Object3D
  visible: boolean
}

interface ContextState {
  selectedAssetId: string | null
  selectedVersionId: number | null
  sidebarOpen: boolean
  searchQuery: string

  selectAsset: (id: string | null) => void
  selectVersion: (id: number | null) => void
  setSidebarOpen: (open: boolean) => void
  setSearchQuery: (q: string) => void

  recentAssets: AssetSummary[]
  addRecentAsset: (asset: AssetSummary) => void
  clearRecentAssets: () => void

  // ── Scene object catalog (M8 T8.6 / T8.7) ─────────────────────────────────
  sceneObjects: SceneObjectEntry[]
  selectedObjectId: string | null

  setSceneObjects: (entries: SceneObjectEntry[]) => void
  addSceneObject: (entry: SceneObjectEntry) => void
  removeSceneObject: (id: string) => void
  setObjectVisibility: (id: string, visible: boolean) => void
  selectObject: (id: string | null) => void
  resetSceneObjects: () => void

  // ── PlastiqEngine Select integration ──────────────────────────────────────
  //
  // The studio's pick-and-inspect toolkit publishes the active Selection
  // discriminated-union value here. Consumers branch by `selection.kind`
  // for kind-aware behavior; ObjectControls reads `selectedObjectId`
  // directly which is mirrored from `selection.object.uuid` whenever the
  // selection points at a tracked SceneObjectEntry.
  //
  // `selectionMode` constrains what kind of selection a pointer event
  // produces (object / mesh / face / point / edge / plane / solid).

  currentSelection: Selection | null
  selectionMode: SelectionMode
  setSelection: (selection: Selection | null) => void
  setSelectionMode: (mode: SelectionMode) => void
  clearSelection: () => void
}

export const useContextStore = create<ContextState>((set) => ({
  selectedAssetId: null,
  selectedVersionId: null,
  sidebarOpen: true,
  searchQuery: '',
  recentAssets: [],
  sceneObjects: [],
  selectedObjectId: null,
  currentSelection: null,
  selectionMode: 'object',

  selectAsset: (id) => set({ selectedAssetId: id, selectedVersionId: null }),
  selectVersion: (id) => set({ selectedVersionId: id }),
  setSidebarOpen: (open) => set({ sidebarOpen: open }),
  setSearchQuery: (q) => set({ searchQuery: q }),

  addRecentAsset: (asset) =>
    set((state) => ({
      recentAssets: [
        asset,
        ...state.recentAssets.filter((a) => a.id !== asset.id),
      ].slice(0, 20),
    })),
  clearRecentAssets: () => set({ recentAssets: [] }),

  setSceneObjects: (sceneObjects) => set({ sceneObjects }),
  addSceneObject: (entry) =>
    set((state) => ({
      sceneObjects: [
        ...state.sceneObjects.filter((e) => e.id !== entry.id),
        entry,
      ],
    })),
  removeSceneObject: (id) =>
    set((state) => ({
      sceneObjects: state.sceneObjects.filter((e) => e.id !== id),
      selectedObjectId:
        state.selectedObjectId === id ? null : state.selectedObjectId,
    })),
  setObjectVisibility: (id, visible) =>
    set((state) => ({
      sceneObjects: state.sceneObjects.map((e) => {
        if (e.id !== id) return e
        // Mutate the live Three.js node so the scene graph picks up
        // visibility changes immediately. The state copy carries the
        // new flag so React subscribers re-render.
        e.object.visible = visible
        return { ...e, visible }
      }),
    })),
  selectObject: (selectedObjectId) => set({ selectedObjectId }),
  resetSceneObjects: () => set({ sceneObjects: [], selectedObjectId: null }),

  setSelection: (selection) =>
    set((state) => {
      // Skip identical re-publishes so downstream effects (gizmo
      // remount, measurement re-derivation) don't churn on no-op picks.
      if (selectionsEqual(state.currentSelection, selection)) return state

      // Mirror the selection's underlying Object3D into the existing
      // selectedObjectId so ObjectControls keeps working unchanged.
      let nextSelectedId = state.selectedObjectId
      if (selection && 'object' in selection && selection.object) {
        const targetUuid = selection.object.uuid
        const tracked = state.sceneObjects.find(
          (e) => e.object.uuid === targetUuid,
        )
        if (tracked) nextSelectedId = tracked.id
      } else if (!selection) {
        nextSelectedId = null
      }

      return { currentSelection: selection, selectedObjectId: nextSelectedId }
    }),
  setSelectionMode: (selectionMode) => set({ selectionMode }),
  clearSelection: () => set({ currentSelection: null }),
}))
