import { create } from 'zustand'
import type { CameraViewDirection } from './rendererStore'

// SPEC-1 M-14, §6.2 — `snapshotPresetStore`.
//
// A snapshot preset bundles a camera pose + render settings + frame size
// so the user can save "the angle I shoot the front render at" once and
// reuse it across re-renders. Snapshots are scoped per asset (an asset's
// "front render" makes no sense for another asset's geometry).
//
// State shape mirrors `positionStore` (per-asset list) but the entries
// carry render-time configuration that `positionStore` deliberately
// omits — tone mapping, output dimensions, transparent background flag,
// and the named camera direction the SnapshotPresetsPanel re-applies via
// `useSetCurrentView`.

export interface SnapshotPresetEntry {
  id: string
  name: string
  /** Optional named camera direction (set when the preset was authored from a viewcube click). */
  cameraDirection: CameraViewDirection | null
  /** Camera world-space position (only used when `cameraDirection` is null). */
  cameraPosition: [number, number, number] | null
  cameraTarget: [number, number, number] | null
  fov: number
  /** Output frame size in pixels. */
  width: number
  height: number
  transparent: boolean
  /** Stored as the render mode key from rendererStore. */
  renderMode: string
  /** Stored as the tone mapping mode key from rendererStore. */
  toneMapping: string
  /** Epoch milliseconds. */
  createdAt: number
  updatedAt: number
}

interface SnapshotPresetState {
  byAsset: Record<string, SnapshotPresetEntry[]>

  list(assetId: string): SnapshotPresetEntry[]
  get(assetId: string, presetId: string): SnapshotPresetEntry | null

  save(
    assetId: string,
    entry: Omit<SnapshotPresetEntry, 'createdAt' | 'updatedAt'>,
  ): SnapshotPresetEntry
  rename(assetId: string, presetId: string, name: string): boolean
  remove(assetId: string, presetId: string): boolean
  setAll(assetId: string, entries: SnapshotPresetEntry[]): void

  reset(): void
}

export const useSnapshotPresetStore = create<SnapshotPresetState>((set, get) => ({
  byAsset: {},

  list(assetId) {
    return get().byAsset[assetId] ?? []
  },

  get(assetId, presetId) {
    return (get().byAsset[assetId] ?? []).find((p) => p.id === presetId) ?? null
  },

  save(assetId, entry) {
    const now = Date.now()
    let saved!: SnapshotPresetEntry
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

  rename(assetId, presetId, name) {
    let mutated = false
    set((state) => {
      const list = state.byAsset[assetId] ?? []
      const idx = list.findIndex((p) => p.id === presetId)
      if (idx === -1) return state
      mutated = true
      const updated: SnapshotPresetEntry = {
        ...list[idx],
        name,
        updatedAt: Date.now(),
      }
      const next = list.map((p) => (p.id === presetId ? updated : p))
      return { byAsset: { ...state.byAsset, [assetId]: next } }
    })
    return mutated
  },

  remove(assetId, presetId) {
    let mutated = false
    set((state) => {
      const list = state.byAsset[assetId] ?? []
      if (!list.some((p) => p.id === presetId)) return state
      mutated = true
      const next = list.filter((p) => p.id !== presetId)
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
