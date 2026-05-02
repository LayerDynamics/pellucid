import { create } from 'zustand'

// SPEC-1 §6.2 stores — `positionStore` (referenced by `usePositionSet`).
//
// Holds named camera positions per asset. Each entry captures:
//   - Camera position + target + up vector (for the orbit controls)
//   - The render mode + tone mapping at the time the user saved the pose
//     (so loading the pose reproduces the visual exactly)
//
// Distinction vs `snapshotPresetStore`: a Position is geometric only
// (camera pose). A SnapshotPreset (M-14) is Position + render settings +
// frame size, and is exclusively used by SnapshotPresetsPanel for batch
// snapshot capture. Position is the lighter unit Studios reuse via
// `usePositionSet`.

export interface PositionEntry {
  id: string
  name: string
  /** Camera world-space position. */
  position: [number, number, number]
  /** Look-at target. */
  target: [number, number, number]
  /** Camera up vector. */
  up: [number, number, number]
  /** Vertical FOV in degrees. */
  fov: number
  /** Epoch milliseconds. */
  createdAt: number
  updatedAt: number
}

interface PositionState {
  /** assetId → ordered list of positions for that asset. */
  byAsset: Record<string, PositionEntry[]>

  list(assetId: string): PositionEntry[]
  get(assetId: string, positionId: string): PositionEntry | null

  save(assetId: string, entry: Omit<PositionEntry, 'createdAt' | 'updatedAt'>): PositionEntry
  rename(assetId: string, positionId: string, name: string): boolean
  remove(assetId: string, positionId: string): boolean
  reorder(assetId: string, fromIndex: number, toIndex: number): boolean

  /** Replace all positions for an asset (used by hydration / import). */
  setAll(assetId: string, entries: PositionEntry[]): void

  reset(): void
}

export const usePositionStore = create<PositionState>((set, get) => ({
  byAsset: {},

  list(assetId) {
    return get().byAsset[assetId] ?? []
  },

  get(assetId, positionId) {
    return (get().byAsset[assetId] ?? []).find((p) => p.id === positionId) ?? null
  },

  save(assetId, entry) {
    const now = Date.now()
    let saved!: PositionEntry
    set((state) => {
      const list = state.byAsset[assetId] ?? []
      const idx = list.findIndex((p) => p.id === entry.id)
      saved =
        idx === -1
          ? { ...entry, createdAt: now, updatedAt: now }
          : { ...list[idx], ...entry, updatedAt: now }
      const next =
        idx === -1
          ? [...list, saved]
          : list.map((p) => (p.id === entry.id ? saved : p))
      return { byAsset: { ...state.byAsset, [assetId]: next } }
    })
    return saved
  },

  rename(assetId, positionId, name) {
    let mutated = false
    set((state) => {
      const list = state.byAsset[assetId] ?? []
      const idx = list.findIndex((p) => p.id === positionId)
      if (idx === -1) return state
      mutated = true
      const updated: PositionEntry = { ...list[idx], name, updatedAt: Date.now() }
      const next = list.map((p) => (p.id === positionId ? updated : p))
      return { byAsset: { ...state.byAsset, [assetId]: next } }
    })
    return mutated
  },

  remove(assetId, positionId) {
    let mutated = false
    set((state) => {
      const list = state.byAsset[assetId] ?? []
      if (!list.some((p) => p.id === positionId)) return state
      mutated = true
      const next = list.filter((p) => p.id !== positionId)
      return { byAsset: { ...state.byAsset, [assetId]: next } }
    })
    return mutated
  },

  reorder(assetId, fromIndex, toIndex) {
    let mutated = false
    set((state) => {
      const list = state.byAsset[assetId] ?? []
      if (
        fromIndex < 0 ||
        toIndex < 0 ||
        fromIndex >= list.length ||
        toIndex >= list.length ||
        fromIndex === toIndex
      ) {
        return state
      }
      mutated = true
      const next = list.slice()
      const [item] = next.splice(fromIndex, 1)
      next.splice(toIndex, 0, item)
      return { byAsset: { ...state.byAsset, [assetId]: next } }
    })
    return mutated
  },

  setAll(assetId, entries) {
    set((state) => ({ byAsset: { ...state.byAsset, [assetId]: entries.slice() } }))
  },

  reset() {
    set({ byAsset: {} })
  },
}))
