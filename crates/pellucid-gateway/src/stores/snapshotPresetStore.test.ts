/* @vitest-environment jsdom */
import { describe, it, expect, beforeEach } from 'vitest'
import {
  useSnapshotPresetStore,
  type SnapshotPresetEntry,
} from './snapshotPresetStore'

const base: Omit<SnapshotPresetEntry, 'createdAt' | 'updatedAt'> = {
  id: 's-1',
  name: 'Front 1080',
  cameraDirection: 'front',
  cameraPosition: null,
  cameraTarget: null,
  fov: 45,
  width: 1920,
  height: 1080,
  transparent: false,
  renderMode: 'solid',
  toneMapping: 'aces',
}

describe('snapshotPresetStore', () => {
  beforeEach(() => {
    useSnapshotPresetStore.getState().reset()
  })

  it('starts empty per asset', () => {
    expect(useSnapshotPresetStore.getState().list('asset-1')).toEqual([])
  })

  it('save() adds a preset and stamps timestamps', () => {
    const before = Date.now()
    const saved = useSnapshotPresetStore.getState().save('asset-1', base)
    expect(saved.id).toBe('s-1')
    expect(saved.createdAt).toBeGreaterThanOrEqual(before)
    expect(saved.cameraDirection).toBe('front')
  })

  it('save() with same id replaces in place', async () => {
    const s = useSnapshotPresetStore.getState()
    const a = s.save('asset-1', base)
    await new Promise((r) => setTimeout(r, 5))
    const b = s.save('asset-1', { ...base, name: 'Front Square', width: 1080, height: 1080 })
    expect(useSnapshotPresetStore.getState().list('asset-1')).toHaveLength(1)
    expect(b.width).toBe(1080)
    expect(b.createdAt).toBe(a.createdAt)
  })

  it('rename() updates only the named preset', () => {
    const s = useSnapshotPresetStore.getState()
    s.save('asset-1', base)
    expect(s.rename('asset-1', 's-1', 'Front HD')).toBe(true)
    expect(useSnapshotPresetStore.getState().list('asset-1')[0].name).toBe('Front HD')
  })

  it('remove() deletes', () => {
    const s = useSnapshotPresetStore.getState()
    s.save('asset-1', base)
    expect(s.remove('asset-1', 's-1')).toBe(true)
    expect(useSnapshotPresetStore.getState().list('asset-1')).toEqual([])
  })

  it('presets are scoped per asset', () => {
    const s = useSnapshotPresetStore.getState()
    s.save('asset-A', base)
    s.save('asset-B', { ...base, id: 's-1', name: 'Other' })
    expect(useSnapshotPresetStore.getState().list('asset-A')[0].name).toBe('Front 1080')
    expect(useSnapshotPresetStore.getState().list('asset-B')[0].name).toBe('Other')
  })

  it('get() looks up by id', () => {
    const s = useSnapshotPresetStore.getState()
    s.save('asset-1', base)
    expect(useSnapshotPresetStore.getState().get('asset-1', 's-1')?.name).toBe('Front 1080')
    expect(useSnapshotPresetStore.getState().get('asset-1', 'ghost')).toBeNull()
  })

  it('setAll() replaces the per-asset list', () => {
    const s = useSnapshotPresetStore.getState()
    s.save('asset-1', base)
    const fresh: SnapshotPresetEntry = {
      ...base,
      id: 'fresh',
      name: 'Fresh',
      createdAt: 1,
      updatedAt: 1,
    }
    s.setAll('asset-1', [fresh])
    expect(useSnapshotPresetStore.getState().list('asset-1').map((p) => p.id)).toEqual(['fresh'])
  })
})
