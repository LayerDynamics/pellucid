/* @vitest-environment jsdom */
import { describe, it, expect, beforeEach } from 'vitest'
import { usePhotoStore, type SnapshotEntry } from './photoStore'

function snap(id: string, role: SnapshotEntry['role'] = 'unset'): SnapshotEntry {
  return {
    id,
    blob: new Blob([new Uint8Array(4)], { type: 'image/png' }),
    width: 256,
    height: 256,
    transparent: false,
    capturedAt: Date.now(),
    role,
  }
}

beforeEach(() => {
  usePhotoStore.getState().reset()
})

describe('photoStore (R-09 / R-10 / R-12 surface)', () => {
  it('starts empty with sane defaults', () => {
    const s = usePhotoStore.getState()
    expect(s.snapshots).toEqual([])
    expect(s.latestSnapshot).toBeNull()
    expect(s.outputWidth).toBe(1920)
    expect(s.outputHeight).toBe(1080)
  })

  it('pushSnapshot prepends and tracks latestSnapshot', () => {
    const s = usePhotoStore.getState()
    s.pushSnapshot(snap('a'))
    s.pushSnapshot(snap('b'))
    const st = usePhotoStore.getState()
    expect(st.snapshots.map((e) => e.id)).toEqual(['b', 'a'])
    expect(st.latestSnapshot?.id).toBe('b')
  })

  it('pushSnapshot caps history at 50 entries', () => {
    const s = usePhotoStore.getState()
    for (let i = 0; i < 60; i++) s.pushSnapshot(snap(`s-${i}`))
    expect(usePhotoStore.getState().snapshots.length).toBe(50)
    expect(usePhotoStore.getState().snapshots[0].id).toBe('s-59')
  })

  it('setSnapshotRole updates only the named entry', () => {
    const s = usePhotoStore.getState()
    s.pushSnapshot(snap('a'))
    s.pushSnapshot(snap('b'))
    s.setSnapshotRole('a', 'thumbnail')
    const list = usePhotoStore.getState().snapshots
    expect(list.find((e) => e.id === 'a')?.role).toBe('thumbnail')
    expect(list.find((e) => e.id === 'b')?.role).toBe('unset')
  })

  it('setSnapshotRole keeps latestSnapshot in sync when the renamed id is the latest', () => {
    const s = usePhotoStore.getState()
    s.pushSnapshot(snap('a'))
    s.setSnapshotRole('a', 'gallery')
    expect(usePhotoStore.getState().latestSnapshot?.role).toBe('gallery')
  })

  it('setSnapshotRole leaves latestSnapshot untouched when a different id is renamed', () => {
    const s = usePhotoStore.getState()
    s.pushSnapshot(snap('older'))
    s.pushSnapshot(snap('latest'))
    s.setSnapshotRole('older', 'thumbnail')
    expect(usePhotoStore.getState().latestSnapshot?.id).toBe('latest')
    expect(usePhotoStore.getState().latestSnapshot?.role).toBe('unset')
  })

  it('removeSnapshot drops the entry from the list', () => {
    const s = usePhotoStore.getState()
    s.pushSnapshot(snap('a'))
    s.pushSnapshot(snap('b'))
    s.removeSnapshot('a')
    expect(usePhotoStore.getState().snapshots.map((e) => e.id)).toEqual(['b'])
  })

  it('removeSnapshot moves latestSnapshot to the next-newest when the latest is removed', () => {
    const s = usePhotoStore.getState()
    s.pushSnapshot(snap('older'))
    s.pushSnapshot(snap('latest'))
    s.removeSnapshot('latest')
    expect(usePhotoStore.getState().latestSnapshot?.id).toBe('older')
  })

  it('removeSnapshot of the only entry sets latestSnapshot to null', () => {
    const s = usePhotoStore.getState()
    s.pushSnapshot(snap('only'))
    s.removeSnapshot('only')
    expect(usePhotoStore.getState().latestSnapshot).toBeNull()
    expect(usePhotoStore.getState().snapshots).toEqual([])
  })

  it('clearSnapshots empties both the list and latestSnapshot', () => {
    const s = usePhotoStore.getState()
    s.pushSnapshot(snap('a'))
    s.pushSnapshot(snap('b'))
    s.clearSnapshots()
    expect(usePhotoStore.getState().snapshots).toEqual([])
    expect(usePhotoStore.getState().latestSnapshot).toBeNull()
  })

  it('setOutputSize writes the photo-mode output dimensions', () => {
    const s = usePhotoStore.getState()
    s.setOutputSize(658, 522)
    expect(usePhotoStore.getState().outputWidth).toBe(658)
    expect(usePhotoStore.getState().outputHeight).toBe(522)
  })

  it('reset() restores defaults', () => {
    const s = usePhotoStore.getState()
    s.pushSnapshot(snap('a', 'thumbnail'))
    s.setOutputSize(800, 600)
    s.reset()
    const st = usePhotoStore.getState()
    expect(st.snapshots).toEqual([])
    expect(st.latestSnapshot).toBeNull()
    expect(st.outputWidth).toBe(1920)
    expect(st.outputHeight).toBe(1080)
  })
})
