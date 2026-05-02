/* @vitest-environment jsdom */
import { describe, it, expect, beforeEach } from 'vitest'
import { usePositionStore, type PositionEntry } from './positionStore'

const baseEntry: Omit<PositionEntry, 'createdAt' | 'updatedAt'> = {
  id: 'p-1',
  name: 'Front',
  position: [0, 0, 5],
  target: [0, 0, 0],
  up: [0, 1, 0],
  fov: 45,
}

describe('positionStore', () => {
  beforeEach(() => {
    usePositionStore.getState().reset()
  })

  it('starts empty', () => {
    expect(usePositionStore.getState().list('asset-1')).toEqual([])
    expect(usePositionStore.getState().get('asset-1', 'p-1')).toBeNull()
  })

  it('save() inserts and stamps timestamps', () => {
    const before = Date.now()
    const saved = usePositionStore.getState().save('asset-1', baseEntry)
    expect(saved.id).toBe('p-1')
    expect(saved.createdAt).toBeGreaterThanOrEqual(before)
    expect(saved.updatedAt).toBeGreaterThanOrEqual(before)
    expect(usePositionStore.getState().list('asset-1')).toHaveLength(1)
  })

  it('save() with the same id updates in place and bumps updatedAt only', async () => {
    const s = usePositionStore.getState()
    const a = s.save('asset-1', baseEntry)
    // Wait long enough that Date.now() advances on every platform.
    await new Promise((r) => setTimeout(r, 5))
    const b = s.save('asset-1', { ...baseEntry, name: 'Renamed' })
    expect(usePositionStore.getState().list('asset-1')).toHaveLength(1)
    expect(b.name).toBe('Renamed')
    expect(b.createdAt).toBe(a.createdAt)
    expect(b.updatedAt).toBeGreaterThanOrEqual(a.updatedAt)
  })

  it('rename() mutates only the named entry', () => {
    const s = usePositionStore.getState()
    s.save('asset-1', baseEntry)
    s.save('asset-1', { ...baseEntry, id: 'p-2', name: 'Side' })
    expect(s.rename('asset-1', 'p-2', 'Right')).toBe(true)
    expect(usePositionStore.getState().get('asset-1', 'p-2')?.name).toBe('Right')
    expect(usePositionStore.getState().get('asset-1', 'p-1')?.name).toBe('Front')
  })

  it('rename() returns false for unknown id', () => {
    const s = usePositionStore.getState()
    s.save('asset-1', baseEntry)
    expect(s.rename('asset-1', 'ghost', 'X')).toBe(false)
  })

  it('remove() removes the entry', () => {
    const s = usePositionStore.getState()
    s.save('asset-1', baseEntry)
    expect(s.remove('asset-1', 'p-1')).toBe(true)
    expect(usePositionStore.getState().list('asset-1')).toHaveLength(0)
  })

  it('reorder() moves entries', () => {
    const s = usePositionStore.getState()
    s.save('asset-1', baseEntry) // p-1
    s.save('asset-1', { ...baseEntry, id: 'p-2', name: 'Side' })
    s.save('asset-1', { ...baseEntry, id: 'p-3', name: 'Back' })
    expect(s.reorder('asset-1', 0, 2)).toBe(true)
    expect(usePositionStore.getState().list('asset-1').map((p) => p.id)).toEqual([
      'p-2',
      'p-3',
      'p-1',
    ])
  })

  it('reorder() rejects out-of-range indices', () => {
    const s = usePositionStore.getState()
    s.save('asset-1', baseEntry)
    expect(s.reorder('asset-1', 0, 5)).toBe(false)
    expect(s.reorder('asset-1', -1, 0)).toBe(false)
    expect(s.reorder('asset-1', 0, 0)).toBe(false)
  })

  it('positions are scoped per asset', () => {
    const s = usePositionStore.getState()
    s.save('asset-A', baseEntry)
    s.save('asset-B', { ...baseEntry, id: 'p-1', name: 'Other' })
    expect(usePositionStore.getState().list('asset-A')[0].name).toBe('Front')
    expect(usePositionStore.getState().list('asset-B')[0].name).toBe('Other')
  })

  it('setAll() replaces wholesale', () => {
    const s = usePositionStore.getState()
    s.save('asset-1', baseEntry)
    const replacement: PositionEntry = {
      ...baseEntry,
      id: 'fresh',
      name: 'Fresh',
      createdAt: 1,
      updatedAt: 1,
    }
    s.setAll('asset-1', [replacement])
    expect(usePositionStore.getState().list('asset-1').map((p) => p.id)).toEqual(['fresh'])
  })
})
