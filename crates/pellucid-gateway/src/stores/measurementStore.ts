import { create } from 'zustand'
import type { Selection } from '@/lib/plastiq-engine/Select/Select'
import type { LengthUnit } from '@/lib/plastiq-engine/Measure/Unit'
import type { BetweenResult } from '@/lib/plastiq-engine/Measure/Between'

// ── Measurement Store ───────────────────────────────────────────────────────
//
// State for the studio's "measure" tool. The tool is a two-step
// interaction: pick a `from` anchor, then a `to` target — once both are
// set, `Viewport/Measure.tsx` runs `measureBetween(from, to)` and
// publishes the live `latestResult` here for the right-side inspector
// and the in-scene overlay.
//
// `mode` controls the active measurement variant:
//   - 'between'      — distance between two selections (default)
//   - 'surface-area' — total surface area of the active selection
//   - 'component'    — bounding-box dimensions of the active selection
//
// `unit` is the operator's preferred display unit and is plumbed into
// every Measure call.
//
// `history` keeps the last N completed measurements so the operator
// can compare measurements without losing them when picking new ones.

export type MeasurementMode = 'between' | 'surface-area' | 'component'

export interface CompletedMeasurement {
  id: string
  result: BetweenResult
  fromKey: string
  toKey: string
  timestamp: number
}

interface MeasurementState {
  active: boolean
  mode: MeasurementMode
  unit: LengthUnit
  fromAnchor: Selection | null
  toAnchor: Selection | null
  latestResult: BetweenResult | null
  history: CompletedMeasurement[]

  setActive: (active: boolean) => void
  setMode: (mode: MeasurementMode) => void
  setUnit: (unit: LengthUnit) => void
  setFromAnchor: (sel: Selection | null) => void
  setToAnchor: (sel: Selection | null) => void
  setLatestResult: (result: BetweenResult | null) => void
  pushHistory: (entry: CompletedMeasurement) => void
  clearHistory: () => void
  reset: () => void
}

const HISTORY_LIMIT = 20

export const useMeasurementStore = create<MeasurementState>((set) => ({
  active: false,
  mode: 'between',
  unit: 'mm',
  fromAnchor: null,
  toAnchor: null,
  latestResult: null,
  history: [],

  setActive: (active) => set({ active }),
  setMode: (mode) => set({ mode }),
  setUnit: (unit) => set({ unit }),
  setFromAnchor: (fromAnchor) => set({ fromAnchor }),
  setToAnchor: (toAnchor) => set({ toAnchor }),
  setLatestResult: (latestResult) => set({ latestResult }),
  pushHistory: (entry) =>
    set((state) => ({
      history: [entry, ...state.history].slice(0, HISTORY_LIMIT),
    })),
  clearHistory: () => set({ history: [] }),
  reset: () =>
    set({
      active: false,
      fromAnchor: null,
      toAnchor: null,
      latestResult: null,
    }),
}))
