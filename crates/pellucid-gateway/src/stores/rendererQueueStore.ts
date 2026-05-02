import { create } from 'zustand'
import { listAssets, type AssetSummary } from '@/lib/plastiq-client'

// SPEC-1 Q-01..Q-04 — Renderer Queue store.
//
// Q-01: queue lists assets where `needs_render = true`.
// Q-02: each asset carries pending|in-progress|done status; the rendererStore
//       owns active render orchestration, this store owns *order*.
// Q-03: advance/prev navigation steps through queue order.
// Q-04: breadcrumb position derived from currentIndex / total.
//
// Hydration: `hydrate()` calls /api/assets with needs_render=true and
// stores the page in `entries`. Re-hydration is the caller's responsibility
// (e.g. on page focus, after a publish completes).
//
// `markCompleted(id)` removes an entry from the queue and adjusts
// currentIndex so the queue keeps pointing at a valid asset (or -1 when
// empty).

export type QueueStatus = 'pending' | 'in_progress' | 'done'

export interface QueueEntry {
  id: string
  name: string
  thumbnail_url: string | null
  status: QueueStatus
}

interface RendererQueueState {
  entries: QueueEntry[]
  currentIndex: number
  loading: boolean
  error: string | null

  hydrate(): Promise<void>
  setEntries(entries: QueueEntry[]): void

  /** Pick the queue entry at this index (or -1 to deselect). */
  setCurrentIndex(index: number): void
  current(): QueueEntry | null

  /** Advance one position. Returns the new current entry, or null at end. */
  advance(): QueueEntry | null
  /** Step back one position. Returns the new current entry, or null at start. */
  prev(): QueueEntry | null
  /** Skip the current entry without changing its status. */
  skip(): QueueEntry | null

  setStatus(id: string, status: QueueStatus): void
  /** Remove an entry; the queue compacts and currentIndex stays valid. */
  markCompleted(id: string): void

  reset(): void
}

function clampIndex(index: number, length: number): number {
  if (length === 0) return -1
  if (index < 0) return 0
  if (index >= length) return length - 1
  return index
}

function summaryToEntry(asset: AssetSummary): QueueEntry {
  return {
    id: asset.id,
    name: asset.name,
    thumbnail_url: asset.thumbnail_url,
    status: asset.render_complete ? 'done' : 'pending',
  }
}

export const useRendererQueueStore = create<RendererQueueState>((set, get) => ({
  entries: [],
  currentIndex: -1,
  loading: false,
  error: null,

  async hydrate() {
    set({ loading: true, error: null })
    try {
      const page = await listAssets({ needs_render: true, page_size: 200 })
      const entries = page.results.map(summaryToEntry)
      set({
        entries,
        currentIndex: entries.length > 0 ? 0 : -1,
        loading: false,
      })
    } catch (err) {
      set({
        loading: false,
        error: err instanceof Error ? err.message : 'Failed to load queue',
      })
    }
  },

  setEntries(entries) {
    set({
      entries,
      currentIndex: entries.length > 0 ? 0 : -1,
    })
  },

  setCurrentIndex(index) {
    const { entries } = get()
    set({ currentIndex: clampIndex(index, entries.length) })
  },

  current() {
    const { entries, currentIndex } = get()
    if (currentIndex < 0 || currentIndex >= entries.length) return null
    return entries[currentIndex]
  },

  advance() {
    const { entries, currentIndex } = get()
    if (entries.length === 0) return null
    const next = currentIndex + 1
    if (next >= entries.length) {
      set({ currentIndex: entries.length - 1 })
      return null
    }
    set({ currentIndex: next })
    return entries[next]
  },

  prev() {
    const { entries, currentIndex } = get()
    if (entries.length === 0) return null
    const prev = currentIndex - 1
    if (prev < 0) {
      set({ currentIndex: 0 })
      return null
    }
    set({ currentIndex: prev })
    return entries[prev]
  },

  skip() {
    return get().advance()
  },

  setStatus(id, status) {
    set((state) => ({
      entries: state.entries.map((e) => (e.id === id ? { ...e, status } : e)),
    }))
  },

  markCompleted(id) {
    set((state) => {
      const idx = state.entries.findIndex((e) => e.id === id)
      if (idx < 0) return state
      const nextEntries = state.entries.filter((e) => e.id !== id)
      let nextCurrent = state.currentIndex
      if (idx < state.currentIndex) {
        nextCurrent = state.currentIndex - 1
      } else if (idx === state.currentIndex) {
        // Stay on the same index so the next pending entry slides under
        // the cursor; if we removed the last entry, clamp to new length.
        nextCurrent = state.currentIndex
      }
      return {
        entries: nextEntries,
        currentIndex: clampIndex(nextCurrent, nextEntries.length),
      }
    })
  },

  reset() {
    set({ entries: [], currentIndex: -1, loading: false, error: null })
  },
}))
