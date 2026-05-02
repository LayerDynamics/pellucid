import { create } from 'zustand'
import type { MaterialDefinition } from '@/lib/plastiq-engine/Material/MaterialSchema'

// SPEC-1 §3.4 M-05, M-06, M-12 — KMP material preset library.
//
// State holds two pools of presets:
//
//   - `bundled`: read-only presets shipped with the app (loaded once at
//     startup from public/assets/kmp/ via `setBundled`). The studio's
//     bundled-presets module wires this on first mount.
//   - `userAuthored`: user-saved materials persisted to IndexedDB.
//     `loadUserPresets` replays from IndexedDB; `saveUserPreset` writes
//     through to IndexedDB before updating the in-memory list.
//
// Browse UI consumes `filtered()` which applies the current category +
// query selection to both pools.
//
// IndexedDB access is delegated to a swappable `kmpStore` interface so
// tests can plug an in-memory implementation without spinning up
// fake-indexeddb.

export type KmpCategory =
  | 'metal'
  | 'wood'
  | 'glass'
  | 'gem'
  | 'fabric'
  | 'plastic'
  | 'paint'
  | 'stone'
  | 'ceramic'
  | 'carbon-fiber'
  | 'organic'
  | 'liquid'
  | 'coating'
  | 'composite'
  | 'showcase'
  | 'uncategorized'

export interface KmpPreset {
  id: string
  name: string
  category: KmpCategory
  thumbnailUrl: string | null
  definition: MaterialDefinition
  source: 'bundled' | 'user'
  tags: string[]
  /** Epoch milliseconds. */
  createdAt: number
  updatedAt: number
}

export interface KmpStorage {
  list(): Promise<KmpPreset[]>
  put(preset: KmpPreset): Promise<void>
  delete(id: string): Promise<void>
}

const DB_NAME = 'plastiq-kmp-library-v1'
const STORE_NAME = 'presets'

class IdbKmpStorage implements KmpStorage {
  private dbPromise: Promise<IDBDatabase> | null = null

  private getDb(): Promise<IDBDatabase> {
    if (this.dbPromise) return this.dbPromise
    if (typeof indexedDB === 'undefined') {
      // Surface a hard error rather than silently returning empty data;
      // the studio's UI gates writes on `isAvailable` to avoid this path.
      return Promise.reject(new Error('IndexedDB not available'))
    }
    this.dbPromise = new Promise<IDBDatabase>((resolve, reject) => {
      const req = indexedDB.open(DB_NAME, 1)
      req.onupgradeneeded = () => {
        const db = req.result
        if (!db.objectStoreNames.contains(STORE_NAME)) {
          db.createObjectStore(STORE_NAME, { keyPath: 'id' })
        }
      }
      req.onsuccess = () => resolve(req.result)
      req.onerror = () => reject(req.error ?? new Error('IndexedDB open failed'))
    })
    return this.dbPromise
  }

  async list(): Promise<KmpPreset[]> {
    const db = await this.getDb()
    return new Promise<KmpPreset[]>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, 'readonly')
      const req = tx.objectStore(STORE_NAME).getAll()
      req.onsuccess = () => resolve((req.result ?? []) as KmpPreset[])
      req.onerror = () => reject(req.error ?? new Error('IDB getAll failed'))
    })
  }

  async put(preset: KmpPreset): Promise<void> {
    const db = await this.getDb()
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, 'readwrite')
      tx.objectStore(STORE_NAME).put(preset)
      tx.oncomplete = () => resolve()
      tx.onerror = () => reject(tx.error ?? new Error('IDB put failed'))
    })
  }

  async delete(id: string): Promise<void> {
    const db = await this.getDb()
    return new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, 'readwrite')
      tx.objectStore(STORE_NAME).delete(id)
      tx.oncomplete = () => resolve()
      tx.onerror = () => reject(tx.error ?? new Error('IDB delete failed'))
    })
  }
}

export const idbKmpStorage: KmpStorage = new IdbKmpStorage()

interface KmpLibraryState {
  bundled: KmpPreset[]
  userAuthored: KmpPreset[]
  query: string
  category: KmpCategory | 'all'
  loading: boolean
  error: string | null
  storage: KmpStorage

  setStorage(storage: KmpStorage): void
  setBundled(presets: KmpPreset[]): void
  setQuery(query: string): void
  setCategory(category: KmpCategory | 'all'): void

  loadUserPresets(): Promise<void>
  saveUserPreset(preset: KmpPreset): Promise<void>
  deleteUserPreset(id: string): Promise<void>

  filtered(): KmpPreset[]
  byId(id: string): KmpPreset | null
}

function matchesQuery(preset: KmpPreset, q: string): boolean {
  if (!q) return true
  const lower = q.toLowerCase()
  if (preset.name.toLowerCase().includes(lower)) return true
  if (preset.tags.some((t) => t.toLowerCase().includes(lower))) return true
  return false
}

export const useKmpLibraryStore = create<KmpLibraryState>((set, get) => ({
  bundled: [],
  userAuthored: [],
  query: '',
  category: 'all',
  loading: false,
  error: null,
  storage: idbKmpStorage,

  setStorage(storage) {
    set({ storage })
  },
  setBundled(presets) {
    set({ bundled: presets })
  },
  setQuery(query) {
    set({ query })
  },
  setCategory(category) {
    set({ category })
  },

  async loadUserPresets() {
    set({ loading: true, error: null })
    try {
      const presets = await get().storage.list()
      set({ userAuthored: presets, loading: false })
    } catch (err) {
      set({
        loading: false,
        error: err instanceof Error ? err.message : 'Failed to load user presets',
      })
    }
  },

  async saveUserPreset(preset) {
    const stamped: KmpPreset = {
      ...preset,
      source: 'user',
      updatedAt: Date.now(),
      createdAt: preset.createdAt || Date.now(),
    }
    await get().storage.put(stamped)
    set((state) => {
      const idx = state.userAuthored.findIndex((p) => p.id === stamped.id)
      const next = idx === -1
        ? [...state.userAuthored, stamped]
        : state.userAuthored.map((p) => (p.id === stamped.id ? stamped : p))
      return { userAuthored: next }
    })
  },

  async deleteUserPreset(id) {
    await get().storage.delete(id)
    set((state) => ({
      userAuthored: state.userAuthored.filter((p) => p.id !== id),
    }))
  },

  filtered() {
    const { bundled, userAuthored, query, category } = get()
    const all = [...bundled, ...userAuthored]
    return all.filter((p) => {
      if (category !== 'all' && p.category !== category) return false
      return matchesQuery(p, query)
    })
  },

  byId(id) {
    const { bundled, userAuthored } = get()
    return bundled.find((p) => p.id === id) ?? userAuthored.find((p) => p.id === id) ?? null
  },
}))
