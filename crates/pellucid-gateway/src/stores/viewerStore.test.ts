/* @vitest-environment jsdom */
import { describe, it, expect, beforeEach } from 'vitest'
import {
  useViewerStore,
  viewerKindForExtension,
  type ViewerFileDescriptor,
} from './viewerStore'

function file(overrides: Partial<ViewerFileDescriptor> = {}): ViewerFileDescriptor {
  return {
    id: overrides.id ?? 'f-1',
    filename: overrides.filename ?? 'thing.stl',
    extension: overrides.extension ?? 'stl',
    sizeBytes: overrides.sizeBytes ?? 1024,
    source: overrides.source ?? '/api/assets/x/versions/1/files/1/stream/',
    zipPath: overrides.zipPath ?? null,
  }
}

describe('viewerKindForExtension', () => {
  it('maps mesh formats', () => {
    expect(viewerKindForExtension('stl')).toBe('mesh')
    expect(viewerKindForExtension('obj')).toBe('mesh')
    expect(viewerKindForExtension('3mf')).toBe('mesh')
    expect(viewerKindForExtension('PLY')).toBe('mesh')
    expect(viewerKindForExtension('glb')).toBe('mesh')
    expect(viewerKindForExtension('GLTF')).toBe('mesh')
  })
  it('maps DXF / zip / text / image', () => {
    expect(viewerKindForExtension('dxf')).toBe('dxf')
    expect(viewerKindForExtension('zip')).toBe('zip')
    expect(viewerKindForExtension('txt')).toBe('text')
    expect(viewerKindForExtension('json')).toBe('text')
    expect(viewerKindForExtension('png')).toBe('image')
    expect(viewerKindForExtension('SVG')).toBe('image')
  })
  it('returns unsupported for unknown extensions', () => {
    expect(viewerKindForExtension('xyz')).toBe('unsupported')
  })
})

describe('viewerStore', () => {
  beforeEach(() => {
    useViewerStore.getState().reset()
  })

  it('starts closed with no current file', () => {
    const s = useViewerStore.getState()
    expect(s.open).toBe(false)
    expect(s.currentFile).toBeNull()
    expect(s.history).toEqual([])
  })

  it('show() opens the panel and records history', () => {
    const s = useViewerStore.getState()
    const f = file()
    s.show(f)
    const st = useViewerStore.getState()
    expect(st.open).toBe(true)
    expect(st.currentFile?.id).toBe(f.id)
    expect(st.history.map((h) => h.id)).toEqual(['f-1'])
  })

  it('show() de-dupes and re-orders history', () => {
    const s = useViewerStore.getState()
    s.show(file({ id: 'a' }))
    s.show(file({ id: 'b' }))
    s.show(file({ id: 'a' }))
    expect(useViewerStore.getState().history.map((h) => h.id)).toEqual(['a', 'b'])
  })

  it('history caps at 25 entries', () => {
    const s = useViewerStore.getState()
    for (let i = 0; i < 30; i++) {
      s.show(file({ id: `f-${i}` }))
    }
    expect(useViewerStore.getState().history).toHaveLength(25)
    expect(useViewerStore.getState().history[0].id).toBe('f-29')
  })

  it('close() clears current but preserves history', () => {
    const s = useViewerStore.getState()
    s.show(file())
    s.close()
    const st = useViewerStore.getState()
    expect(st.open).toBe(false)
    expect(st.currentFile).toBeNull()
    expect(st.history).toHaveLength(1)
  })

  it('toggleZipFolder + isZipFolderExpanded round-trip', () => {
    const s = useViewerStore.getState()
    expect(s.isZipFolderExpanded('zip-1', 'a')).toBe(false)
    s.toggleZipFolder('zip-1', 'a')
    expect(useViewerStore.getState().isZipFolderExpanded('zip-1', 'a')).toBe(true)
    s.toggleZipFolder('zip-1', 'a')
    expect(useViewerStore.getState().isZipFolderExpanded('zip-1', 'a')).toBe(false)
  })

  it('zip expansion is scoped per archive', () => {
    const s = useViewerStore.getState()
    s.toggleZipFolder('zip-A', 'folder')
    expect(s.isZipFolderExpanded('zip-A', 'folder')).toBe(true)
    expect(s.isZipFolderExpanded('zip-B', 'folder')).toBe(false)
  })

  it('setZipExpansion seeds the expanded set wholesale', () => {
    const s = useViewerStore.getState()
    s.setZipExpansion('zip-1', ['a', 'a/b', 'c'])
    expect(s.isZipFolderExpanded('zip-1', 'a')).toBe(true)
    expect(s.isZipFolderExpanded('zip-1', 'a/b')).toBe(true)
    expect(s.isZipFolderExpanded('zip-1', 'd')).toBe(false)
  })
})
