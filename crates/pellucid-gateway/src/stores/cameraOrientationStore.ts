import { create } from 'zustand'
import type { CameraViewDirection } from './rendererStore'

// SPEC-1 §6.2 — `cameraOrientationStore`.
//
// Tracks the current named view direction (front/iso-front-left/etc.) plus
// an undo/redo history of view changes. Pairs with `useSetCurrentView`:
// the hook flips the camera; this store records *that* flip so the user
// can undo their last view change without unwinding any other state.
//
// The history is a bounded ring (default 50 entries). `undo()` walks
// backwards through the ring and republishes the prior direction.

const DEFAULT_LIMIT = 50

interface CameraOrientationState {
  current: CameraViewDirection | null
  /** Past entries newest-last. */
  past: CameraViewDirection[]
  /** Future entries newest-first (populated when undo() runs). */
  future: CameraViewDirection[]
  limit: number

  setLimit(limit: number): void
  /** Record a new direction at the head of history. Replaces redo stack. */
  push(direction: CameraViewDirection): void
  undo(): CameraViewDirection | null
  redo(): CameraViewDirection | null
  canUndo(): boolean
  canRedo(): boolean
  reset(): void
}

export const useCameraOrientationStore = create<CameraOrientationState>((set, get) => ({
  current: null,
  past: [],
  future: [],
  limit: DEFAULT_LIMIT,

  setLimit(limit) {
    set({ limit: Math.max(1, limit) })
  },

  push(direction) {
    set((state) => {
      if (state.current === direction) return state
      const past =
        state.current === null
          ? state.past
          : [...state.past, state.current].slice(-state.limit)
      return {
        current: direction,
        past,
        future: [],
      }
    })
  },

  undo() {
    const { past, current, future } = get()
    if (past.length === 0) return null
    const prev = past[past.length - 1]
    const nextPast = past.slice(0, -1)
    const nextFuture = current !== null ? [current, ...future] : future
    set({ current: prev, past: nextPast, future: nextFuture })
    return prev
  },

  redo() {
    const { future, current, past } = get()
    if (future.length === 0) return null
    const next = future[0]
    const nextFuture = future.slice(1)
    const nextPast = current !== null ? [...past, current] : past
    set({ current: next, past: nextPast, future: nextFuture })
    return next
  },

  canUndo() {
    return get().past.length > 0
  },

  canRedo() {
    return get().future.length > 0
  },

  reset() {
    set({ current: null, past: [], future: [] })
  },
}))
