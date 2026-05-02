/* @vitest-environment jsdom */
import { describe, it, expect, beforeEach } from 'vitest'
import {
  useMaterialStudioStore,
  type MaterialLayer,
} from './materialStudioStore'
import {
  createDefaultMaterialDefinition,
  type MaterialDefinition,
} from '@/lib/plastiq-engine/Material/MaterialSchema'

function layer(id: string, overrides: Partial<MaterialLayer> = {}): MaterialLayer {
  return {
    id,
    name: id,
    scope: 'base',
    enabled: true,
    blendMode: 'normal',
    opacity: 1,
    maskUrl: null,
    adjustment: null,
    label: null,
    parentId: null,
    params: {},
    ...overrides,
  }
}

describe('materialStudioStore', () => {
  beforeEach(() => {
    useMaterialStudioStore.getState().reset()
  })

  it('starts with closed studio and base inspector tab', () => {
    const s = useMaterialStudioStore.getState()
    expect(s.open).toBe(false)
    expect(s.inspectorTab).toBe('base')
    expect(s.previewGeometry).toBe('sphere')
    expect(s.layers).toEqual([])
    expect(s.dirty).toBe(false)
  })

  it('inspector tab + preview + hdri setters mutate state', () => {
    const s = useMaterialStudioStore.getState()
    s.setInspectorTab('layers')
    expect(useMaterialStudioStore.getState().inspectorTab).toBe('layers')
    s.setPreviewGeometry('torus')
    expect(useMaterialStudioStore.getState().previewGeometry).toBe('torus')
    s.setHdriPreset('outdoor')
    expect(useMaterialStudioStore.getState().hdriPresetId).toBe('outdoor')
    expect(useMaterialStudioStore.getState().dirty).toBe(true)
  })

  it('loadDefinition seeds the working def and clears dirty', () => {
    const s = useMaterialStudioStore.getState()
    const def: MaterialDefinition = { ...createDefaultMaterialDefinition(), color: '#ff0000' }
    s.loadDefinition(def)
    expect(useMaterialStudioStore.getState().definition).toEqual(def)
    expect(useMaterialStudioStore.getState().dirty).toBe(false)
  })

  it('patchDefinition merges into the working def and marks dirty', () => {
    const s = useMaterialStudioStore.getState()
    const def: MaterialDefinition = { ...createDefaultMaterialDefinition(), color: '#ff0000' }
    s.loadDefinition(def)
    s.patchDefinition({ metalness: 0.5 })
    expect(useMaterialStudioStore.getState().definition?.color).toBe('#ff0000')
    expect(useMaterialStudioStore.getState().definition?.metalness).toBe(0.5)
    expect(useMaterialStudioStore.getState().dirty).toBe(true)
  })

  it('addLayer appends and selects the new layer', () => {
    const s = useMaterialStudioStore.getState()
    s.addLayer(layer('a'))
    s.addLayer(layer('b'))
    const st = useMaterialStudioStore.getState()
    expect(st.layers.map((l) => l.id)).toEqual(['a', 'b'])
    expect(st.activeLayerId).toBe('b')
    expect(st.dirty).toBe(true)
  })

  it('updateLayer patches a single layer', () => {
    const s = useMaterialStudioStore.getState()
    s.addLayer(layer('a'))
    s.updateLayer('a', { opacity: 0.5, blendMode: 'multiply' })
    const updated = useMaterialStudioStore.getState().layers[0]
    expect(updated.opacity).toBe(0.5)
    expect(updated.blendMode).toBe('multiply')
  })

  it('removeLayer drops and clears active when applicable', () => {
    const s = useMaterialStudioStore.getState()
    s.addLayer(layer('a'))
    s.addLayer(layer('b'))
    s.setActiveLayer('b')
    s.removeLayer('b')
    expect(useMaterialStudioStore.getState().layers.map((l) => l.id)).toEqual(['a'])
    expect(useMaterialStudioStore.getState().activeLayerId).toBeNull()
  })

  it('reorderLayers moves entries; rejects bad indices', () => {
    const s = useMaterialStudioStore.getState()
    s.addLayer(layer('a'))
    s.addLayer(layer('b'))
    s.addLayer(layer('c'))
    expect(s.reorderLayers(0, 2)).toBe(true)
    expect(useMaterialStudioStore.getState().layers.map((l) => l.id)).toEqual(['b', 'c', 'a'])
    expect(s.reorderLayers(0, 0)).toBe(false)
    expect(s.reorderLayers(0, 99)).toBe(false)
  })

  it('markClean clears the dirty flag without touching anything else', () => {
    const s = useMaterialStudioStore.getState()
    s.addLayer(layer('a'))
    expect(useMaterialStudioStore.getState().dirty).toBe(true)
    s.markClean()
    expect(useMaterialStudioStore.getState().dirty).toBe(false)
    expect(useMaterialStudioStore.getState().layers).toHaveLength(1)
  })
})
