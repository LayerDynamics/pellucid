import { create } from 'zustand'

export type LightingPreset = 'studio' | 'outdoor' | 'dramatic' | 'soft' | 'custom'
export type BackgroundType = 'solid' | 'gradient' | 'hdri' | 'transparent'

/**
 * SPEC-1 R-09 — snapshot role tags. Each captured snapshot is staged as
 * either a `gallery` image (multiple allowed per asset) or the single
 * `thumbnail` (asset-detail card). `unset` is the captured-but-unrouted
 * default the user picks from. R-12's single-thumbnail-guard rejects
 * publish when more than one staged snapshot carries the `thumbnail`
 * role; the role lives on the SnapshotEntry rather than in publish-time
 * arguments so the gallery UI can show the chosen role per row.
 */
export type SnapshotRole = 'unset' | 'gallery' | 'thumbnail'

export interface SnapshotEntry {
  /** Stable identifier — typically `Date.now().toString(36)`. */
  id: string
  /** PNG blob holding the rendered pixels. */
  blob: Blob
  /** Width × height the snapshot was taken at. */
  width: number
  height: number
  /** Whether the snapshot was captured against a transparent background. */
  transparent: boolean
  /** Capture timestamp in milliseconds since epoch. */
  capturedAt: number
  /** Publish-time role tag. Defaults to 'unset' on capture. */
  role: SnapshotRole
}

interface PhotoState {
  lightingPreset: LightingPreset
  backgroundType: BackgroundType
  backgroundColor: string
  backgroundGradientStart: string
  backgroundGradientEnd: string
  shadowEnabled: boolean
  shadowOpacity: number
  bloomEnabled: boolean
  bloomStrength: number
  ssaoEnabled: boolean
  outputWidth: number
  outputHeight: number

  /** Most-recent snapshot (also pushed onto `snapshots`). */
  latestSnapshot: SnapshotEntry | null
  /** Newest-first history of snapshots from this session. */
  snapshots: SnapshotEntry[]

  setLightingPreset: (preset: LightingPreset) => void
  setBackgroundType: (type: BackgroundType) => void
  setBackgroundColor: (color: string) => void
  setBackgroundGradient: (start: string, end: string) => void
  setShadowEnabled: (enabled: boolean) => void
  setShadowOpacity: (opacity: number) => void
  setBloomEnabled: (enabled: boolean) => void
  setBloomStrength: (strength: number) => void
  setSsaoEnabled: (enabled: boolean) => void
  setOutputSize: (width: number, height: number) => void
  pushSnapshot: (entry: SnapshotEntry) => void
  /** Re-tag an existing snapshot's role (R-09 / R-12). */
  setSnapshotRole: (id: string, role: SnapshotRole) => void
  /** Drop a snapshot from the gallery (R-10). */
  removeSnapshot: (id: string) => void
  /**
   * Replace the bytes of a captured snapshot in-place — used by R-16
   * (background removal) and R-17 (rotate). Optional width/height
   * override the recorded dimensions; pass when the new blob is a
   * different size (rotation flips W↔H, BG removal preserves them).
   */
  updateSnapshotBlob: (
    id: string,
    blob: Blob,
    opts?: { width?: number; height?: number; transparent?: boolean },
  ) => void
  clearSnapshots: () => void
  reset: () => void
}

const defaults = {
  lightingPreset: 'studio' as LightingPreset,
  backgroundType: 'solid' as BackgroundType,
  backgroundColor: '#ffffff',
  backgroundGradientStart: '#f0f0f0',
  backgroundGradientEnd: '#e0e0e0',
  shadowEnabled: true,
  shadowOpacity: 0.3,
  bloomEnabled: false,
  bloomStrength: 0.5,
  ssaoEnabled: true,
  outputWidth: 1920,
  outputHeight: 1080,
  latestSnapshot: null as SnapshotEntry | null,
  snapshots: [] as SnapshotEntry[],
}

export const usePhotoStore = create<PhotoState>((set) => ({
  ...defaults,

  setLightingPreset: (lightingPreset) => set({ lightingPreset }),
  setBackgroundType: (backgroundType) => set({ backgroundType }),
  setBackgroundColor: (backgroundColor) => set({ backgroundColor }),
  setBackgroundGradient: (backgroundGradientStart, backgroundGradientEnd) =>
    set({ backgroundGradientStart, backgroundGradientEnd }),
  setShadowEnabled: (shadowEnabled) => set({ shadowEnabled }),
  setShadowOpacity: (shadowOpacity) => set({ shadowOpacity }),
  setBloomEnabled: (bloomEnabled) => set({ bloomEnabled }),
  setBloomStrength: (bloomStrength) => set({ bloomStrength }),
  setSsaoEnabled: (ssaoEnabled) => set({ ssaoEnabled }),
  setOutputSize: (outputWidth, outputHeight) => set({ outputWidth, outputHeight }),
  pushSnapshot: (entry) =>
    set((state) => ({
      latestSnapshot: entry,
      snapshots: [entry, ...state.snapshots].slice(0, 50),
    })),
  setSnapshotRole: (id, role) =>
    set((state) => {
      const next = state.snapshots.map((s) =>
        s.id === id ? { ...s, role } : s,
      )
      const latest =
        state.latestSnapshot && state.latestSnapshot.id === id
          ? { ...state.latestSnapshot, role }
          : state.latestSnapshot
      return { snapshots: next, latestSnapshot: latest }
    }),
  removeSnapshot: (id) =>
    set((state) => {
      const next = state.snapshots.filter((s) => s.id !== id)
      const latest =
        state.latestSnapshot && state.latestSnapshot.id === id
          ? next[0] ?? null
          : state.latestSnapshot
      return { snapshots: next, latestSnapshot: latest }
    }),
  updateSnapshotBlob: (id, blob, opts = {}) =>
    set((state) => {
      const patch = (entry: SnapshotEntry): SnapshotEntry => ({
        ...entry,
        blob,
        width: opts.width ?? entry.width,
        height: opts.height ?? entry.height,
        transparent:
          opts.transparent === undefined ? entry.transparent : opts.transparent,
      })
      const next = state.snapshots.map((s) => (s.id === id ? patch(s) : s))
      const latest =
        state.latestSnapshot && state.latestSnapshot.id === id
          ? patch(state.latestSnapshot)
          : state.latestSnapshot
      return { snapshots: next, latestSnapshot: latest }
    }),
  clearSnapshots: () => set({ latestSnapshot: null, snapshots: [] }),
  reset: () => set(defaults),
}))
