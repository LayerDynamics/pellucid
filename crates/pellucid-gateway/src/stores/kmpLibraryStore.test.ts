/* @vitest-environment jsdom */
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { useKmpLibraryStore, type KmpPreset, type KmpStorage } from './kmpLibraryStore'
import { createDefaultMaterialDefinition } from '@/lib/plastiq-engine/Material/MaterialSchema'

function makePreset(overrides: Partial<KmpPreset> = {}): KmpPreset {
  return {
    id: overrides.id ?? 'p-1',
    name: overrides.name ?? 'Test',
    category: overrides.category ?? 'metal',
    thumbnailUrl: overrides.thumbnailUrl ?? null,
    definition: { ...createDefaultMaterialDefinition(), color: '#888888' },
    source: overrides.source ?? 'user',
    tags: overrides.tags ?? [],
    createdAt: overrides.createdAt ?? 1000,
    updatedAt: overrides.updatedAt ?? 1000,
  }
}

class MemKmpStorage implements KmpStorage {
  private rows = new Map<string, KmpPreset>()
  list = vi.fn(async () => Array.from(this.rows.values()))
  put = vi.fn(async (preset: KmpPreset) => {
    this.rows.set(preset.id, preset)
  })
  delete = vi.fn(async (id: string) => {
    this.rows.delete(id)
  })
}

describe('kmpLibraryStore', () => {
  let mem: MemKmpStorage

  beforeEach(() => {
    mem = new MemKmpStorage()
    useKmpLibraryStore.setState({
      bundled: [],
      userAuthored: [],
      query: '',
      category: 'all',
      loading: false,
      error: null,
      storage: mem,
    })
  })

  it('setBundled stores read-only presets', () => {
    const s = useKmpLibraryStore.getState()
    s.setBundled([makePreset({ id: 'b-1', source: 'bundled' })])
    expect(useKmpLibraryStore.getState().bundled).toHaveLength(1)
  })

  it('loadUserPresets reads from storage', async () => {
    await mem.put(makePreset({ id: 'u-1' }))
    const s = useKmpLibraryStore.getState()
    await s.loadUserPresets()
    expect(useKmpLibraryStore.getState().userAuthored).toHaveLength(1)
    expect(mem.list).toHaveBeenCalled()
  })

  it('saveUserPreset writes through and updates list', async () => {
    const s = useKmpLibraryStore.getState()
    await s.saveUserPreset(makePreset({ id: 'u-1', name: 'A' }))
    expect(mem.put).toHaveBeenCalledTimes(1)
    expect(useKmpLibraryStore.getState().userAuthored).toHaveLength(1)
    expect(useKmpLibraryStore.getState().userAuthored[0].source).toBe('user')

    // Re-saving the same id replaces, not appends.
    await s.saveUserPreset(makePreset({ id: 'u-1', name: 'A2' }))
    expect(useKmpLibraryStore.getState().userAuthored).toHaveLength(1)
    expect(useKmpLibraryStore.getState().userAuthored[0].name).toBe('A2')
  })

  it('deleteUserPreset removes from both storage and list', async () => {
    const s = useKmpLibraryStore.getState()
    await s.saveUserPreset(makePreset({ id: 'u-1' }))
    await s.deleteUserPreset('u-1')
    expect(mem.delete).toHaveBeenCalledWith('u-1')
    expect(useKmpLibraryStore.getState().userAuthored).toHaveLength(0)
  })

  it('filtered() merges bundled + user and applies category filter', () => {
    const s = useKmpLibraryStore.getState()
    s.setBundled([
      makePreset({ id: 'b-metal', category: 'metal', source: 'bundled' }),
      makePreset({ id: 'b-wood', category: 'wood', source: 'bundled' }),
    ])
    useKmpLibraryStore.setState({
      userAuthored: [makePreset({ id: 'u-glass', category: 'glass' })],
    })

    expect(useKmpLibraryStore.getState().filtered().map((p) => p.id).sort()).toEqual([
      'b-metal',
      'b-wood',
      'u-glass',
    ])

    s.setCategory('metal')
    expect(useKmpLibraryStore.getState().filtered().map((p) => p.id)).toEqual(['b-metal'])
  })

  it('filtered() applies query filter against name and tags', () => {
    const s = useKmpLibraryStore.getState()
    s.setBundled([
      makePreset({ id: 'b-1', name: 'Brushed Aluminum', source: 'bundled' }),
      makePreset({ id: 'b-2', name: 'Matte Plastic', tags: ['rubber'], source: 'bundled' }),
    ])
    s.setQuery('alum')
    expect(useKmpLibraryStore.getState().filtered().map((p) => p.id)).toEqual(['b-1'])
    s.setQuery('rubber')
    expect(useKmpLibraryStore.getState().filtered().map((p) => p.id)).toEqual(['b-2'])
  })

  it('byId looks up across both pools', () => {
    const s = useKmpLibraryStore.getState()
    s.setBundled([makePreset({ id: 'b', source: 'bundled' })])
    useKmpLibraryStore.setState({ userAuthored: [makePreset({ id: 'u' })] })
    expect(useKmpLibraryStore.getState().byId('b')?.id).toBe('b')
    expect(useKmpLibraryStore.getState().byId('u')?.id).toBe('u')
    expect(useKmpLibraryStore.getState().byId('ghost')).toBeNull()
  })

  it('loadUserPresets captures error on storage failure', async () => {
    mem.list = vi.fn(async () => {
      throw new Error('idb dead')
    })
    await useKmpLibraryStore.getState().loadUserPresets()
    expect(useKmpLibraryStore.getState().error).toBe('idb dead')
    expect(useKmpLibraryStore.getState().loading).toBe(false)
  })
})
