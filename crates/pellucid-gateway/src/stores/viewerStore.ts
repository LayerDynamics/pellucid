import { create } from 'zustand'

// SPEC-1 M3, F-04, F-12 — file viewer store.
//
// Owns the file viewer panel's selection + navigation state. The viewer
// panel reads `currentFile` to decide which inline viewer to mount (STL
// inline, DXF, plain text, zip-tree). Zip-archive contents have their own
// expansion state per archive — we keep it on this store so reopening the
// same zip remembers which folders were open.
//
// `setCurrentFile` accepts the full descriptor — caller-side resolution
// from an AssetFile + ZipEntry is the responsibility of FileViewerPanel.

export type ViewerKind = 'mesh' | 'dxf' | 'text' | 'zip' | 'image' | 'unsupported'

export interface ViewerFileDescriptor {
  /** Globally-unique file id within an asset version (or `${zipFileId}:${zipEntryPath}`). */
  id: string
  /** Display filename (with extension). */
  filename: string
  /** Extension (lowercased, no leading dot). */
  extension: string
  /** Bytes when known (zip entries can be 0 if not yet read). */
  sizeBytes: number | null
  /** URL or stream-token the viewer fetches from. */
  source: string
  /** When the file is a zip entry, the path within the archive. */
  zipPath: string | null
}

interface ViewerState {
  open: boolean
  currentFile: ViewerFileDescriptor | null
  /** zipFileId → set of expanded folder paths (no-trailing-slash). */
  zipExpansion: Record<string, Set<string>>
  /** Most-recently viewed history (newest-first, capped at 25). */
  history: ViewerFileDescriptor[]

  setOpen(open: boolean): void
  show(file: ViewerFileDescriptor): void
  close(): void

  /** Toggle expansion of a folder path inside a zip archive. */
  toggleZipFolder(zipFileId: string, path: string): void
  isZipFolderExpanded(zipFileId: string, path: string): boolean
  setZipExpansion(zipFileId: string, paths: string[]): void

  reset(): void
}

const HISTORY_LIMIT = 25

export function viewerKindForExtension(ext: string): ViewerKind {
  const lower = ext.toLowerCase()
  if (['stl', 'obj', '3mf', 'ply', 'glb', 'gltf'].includes(lower)) return 'mesh'
  if (lower === 'dxf') return 'dxf'
  if (lower === 'zip') return 'zip'
  if (
    [
      'txt', 'md', 'log', 'json', 'xml', 'yaml', 'yml', 'csv', 'tsv',
      'js', 'jsx', 'ts', 'tsx', 'py', 'rs', 'go', 'java', 'cpp', 'c',
      'h', 'hpp', 'cs', 'rb', 'sh', 'env', 'gitignore', 'cfg', 'ini',
    ].includes(lower)
  ) return 'text'
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp', 'svg'].includes(lower)) return 'image'
  return 'unsupported'
}

export const useViewerStore = create<ViewerState>((set, get) => ({
  open: false,
  currentFile: null,
  zipExpansion: {},
  history: [],

  setOpen(open) {
    set({ open })
  },

  show(file) {
    set((state) => ({
      open: true,
      currentFile: file,
      history: [file, ...state.history.filter((h) => h.id !== file.id)].slice(0, HISTORY_LIMIT),
    }))
  },

  close() {
    set({ open: false, currentFile: null })
  },

  toggleZipFolder(zipFileId, path) {
    set((state) => {
      const current = state.zipExpansion[zipFileId] ?? new Set<string>()
      const next = new Set(current)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return { zipExpansion: { ...state.zipExpansion, [zipFileId]: next } }
    })
  },

  isZipFolderExpanded(zipFileId, path) {
    const set = get().zipExpansion[zipFileId]
    return set ? set.has(path) : false
  },

  setZipExpansion(zipFileId, paths) {
    set((state) => ({
      zipExpansion: { ...state.zipExpansion, [zipFileId]: new Set(paths) },
    }))
  },

  reset() {
    set({ open: false, currentFile: null, zipExpansion: {}, history: [] })
  },
}))
