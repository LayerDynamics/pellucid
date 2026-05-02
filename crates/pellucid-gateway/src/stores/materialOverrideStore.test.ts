// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const STORAGE_KEY = 'plastiq.materialOverrides.v1'

async function freshStore() {
  // Clear module cache so the store picks up whatever's in localStorage
  // when the module is first imported. This is the only reliable way to
  // exercise the load-time seed merge.
  const mod = await import('./materialOverrideStore')
  // Reset to a clean instance — the actual store is module-scoped, so we
  // must rebuild the persisted state by re-calling its setters.
  mod.useMaterialOverrideStore.setState({
    overrides: {},
    labels: {},
    activeOverrideKey: null,
    savedBaseline: {},
  })
  return mod
}

beforeEach(() => {
  window.localStorage.clear()
})

afterEach(() => {
  window.localStorage.clear()
})

describe('materialOverrideStore — storage key rename', () => {
  it('persists under plastiq.materialOverrides.v1, never the legacy defcad key', async () => {
    const { useMaterialOverrideStore } = await freshStore()
    useMaterialOverrideStore
      .getState()
      .setOverrideField('builtin:plastic', 'Plastic Material', 'exposure', 1.7)

    const stored = window.localStorage.getItem(STORAGE_KEY)
    expect(stored).not.toBeNull()
    const parsed = JSON.parse(stored!) as {
      overrides: Record<string, { exposure?: number }>
    }
    expect(parsed.overrides['builtin:plastic'].exposure).toBe(1.7)
    expect(window.localStorage.getItem('defcad.materialOverrides.v1')).toBeNull()
  })

  it('exports the storage key constant for callers that need it', async () => {
    const { MATERIAL_OVERRIDE_STORAGE_KEY } = await freshStore()
    expect(MATERIAL_OVERRIDE_STORAGE_KEY).toBe('plastiq.materialOverrides.v1')
  })
})

describe('materialOverrideStore — override CRUD', () => {
  it('sets and reads back fields per material', async () => {
    const { useMaterialOverrideStore } = await freshStore()
    const s = useMaterialOverrideStore.getState()
    s.setOverrideField('kmp:abc', 'My Preset', 'exposure', 2.1)
    s.setOverrideField('kmp:abc', 'My Preset', 'bloomEnabled', true)
    const stored = useMaterialOverrideStore.getState().getOverride('kmp:abc')
    expect(stored).toEqual({ exposure: 2.1, bloomEnabled: true })
    expect(useMaterialOverrideStore.getState().hasOverride('kmp:abc')).toBe(true)
    expect(useMaterialOverrideStore.getState().hasField('kmp:abc', 'bloomEnabled')).toBe(true)
    expect(useMaterialOverrideStore.getState().getOverrideLabel('kmp:abc')).toBe('My Preset')
  })

  it('clearOverrideField removes only that field; clearing the last field deletes the record', async () => {
    const { useMaterialOverrideStore } = await freshStore()
    const s = useMaterialOverrideStore.getState()
    s.setOverrideField('kmp:abc', 'My Preset', 'exposure', 2.1)
    s.setOverrideField('kmp:abc', 'My Preset', 'bloomEnabled', true)
    s.clearOverrideField('kmp:abc', 'bloomEnabled')
    expect(useMaterialOverrideStore.getState().getOverride('kmp:abc')).toEqual({ exposure: 2.1 })
    s.clearOverrideField('kmp:abc', 'exposure')
    expect(useMaterialOverrideStore.getState().getOverride('kmp:abc')).toBeNull()
    expect(useMaterialOverrideStore.getState().getOverrideLabel('kmp:abc')).toBeNull()
  })

  it('clearOverride wipes the entire record and label', async () => {
    const { useMaterialOverrideStore } = await freshStore()
    const s = useMaterialOverrideStore.getState()
    s.setOverrideField('kmp:abc', 'My Preset', 'exposure', 2.1)
    s.setOverrideField('kmp:abc', 'My Preset', 'bloomEnabled', true)
    s.clearOverride('kmp:abc')
    expect(useMaterialOverrideStore.getState().getOverride('kmp:abc')).toBeNull()
    expect(useMaterialOverrideStore.getState().getOverrideLabel('kmp:abc')).toBeNull()
  })

  it('persists changes through localStorage on every setter call', async () => {
    const { useMaterialOverrideStore } = await freshStore()
    useMaterialOverrideStore
      .getState()
      .setOverrideField('builtin:toon', 'Toon', 'bloomStrength', 0.8)
    const parsed = JSON.parse(window.localStorage.getItem(STORAGE_KEY)!) as {
      overrides: Record<string, { bloomStrength?: number }>
      labels: Record<string, string>
    }
    expect(parsed.overrides['builtin:toon']).toEqual({ bloomStrength: 0.8 })
    expect(parsed.labels['builtin:toon']).toBe('Toon')
  })
})

describe('materialOverrideStore — baseline lifecycle', () => {
  it('captures and consumes baseline values per field exactly once', async () => {
    const { useMaterialOverrideStore } = await freshStore()
    const s = useMaterialOverrideStore.getState()
    s._captureBaseline('exposure', 1.0)
    s._captureBaseline('bloomEnabled', false)
    expect(useMaterialOverrideStore.getState()._consumeBaseline('exposure')).toBe(1.0)
    expect(useMaterialOverrideStore.getState()._consumeBaseline('exposure')).toBeUndefined()
    expect(useMaterialOverrideStore.getState()._consumeBaseline('bloomEnabled')).toBe(false)
  })

  it('_clearBaseline drops every captured baseline', async () => {
    const { useMaterialOverrideStore } = await freshStore()
    const s = useMaterialOverrideStore.getState()
    s._captureBaseline('exposure', 1.0)
    s._captureBaseline('bloomEnabled', false)
    s._clearBaseline()
    expect(useMaterialOverrideStore.getState().savedBaseline).toEqual({})
  })

  it('tracks the active material key across switches', async () => {
    const { useMaterialOverrideStore } = await freshStore()
    const s = useMaterialOverrideStore.getState()
    s._setActiveKey('kmp:abc')
    expect(useMaterialOverrideStore.getState().activeOverrideKey).toBe('kmp:abc')
    s._setActiveKey('builtin:toon')
    expect(useMaterialOverrideStore.getState().activeOverrideKey).toBe('builtin:toon')
    s._setActiveKey(null)
    expect(useMaterialOverrideStore.getState().activeOverrideKey).toBeNull()
  })
})

describe('materialOverrideStore — buildDirectionalLightPatch', () => {
  it('joins flat key/fill/back light fields back into DirectionalLightConfig patches', async () => {
    const { buildDirectionalLightPatch } = await freshStore()
    const override = {
      keyLightIntensity: 2,
      keyLightColor: '#ffeebb',
      keyLightAngleDeg: 30,
      // No keyLightElevationDeg — must be omitted from the patch.
      fillLightIntensity: 0.6,
      backLightAngleDeg: 200,
    } as const
    expect(buildDirectionalLightPatch(override, 'keyLight')).toEqual({
      intensity: 2,
      color: '#ffeebb',
      angleDeg: 30,
    })
    expect(buildDirectionalLightPatch(override, 'fillLight')).toEqual({
      intensity: 0.6,
    })
    expect(buildDirectionalLightPatch(override, 'backLight')).toEqual({
      angleDeg: 200,
    })
  })

  it('returns an empty patch when no light fields are present', async () => {
    const { buildDirectionalLightPatch } = await freshStore()
    expect(buildDirectionalLightPatch({}, 'keyLight')).toEqual({})
  })
})

describe('materialOverrideStore — seed merge on first load', () => {
  it('seeds PLASTIQ STANDARD BLACK with exposure 2.5 when storage is empty', async () => {
    // Force a fresh module load so the seed merge runs against an empty
    // localStorage.
    window.localStorage.clear()
    vi.resetModules()
    const mod = await import('./materialOverrideStore')
    const seedKey = 'kmp:7c5e8a1f-4d2b-4e9a-b8c3-f1d2e3a4b5b1'
    const stored = window.localStorage.getItem(STORAGE_KEY)
    expect(stored).not.toBeNull()
    const parsed = JSON.parse(stored!) as {
      overrides: Record<string, { exposure?: number }>
      labels: Record<string, string>
      seedVersion: number
    }
    expect(parsed.overrides[seedKey]?.exposure).toBe(2.5)
    expect(parsed.labels[seedKey]).toBe('PLASTIQ STANDARD BLACK')
    expect(parsed.seedVersion).toBe(1)
    expect(mod.useMaterialOverrideStore.getState().getOverride(seedKey)).toEqual({
      exposure: 2.5,
    })
  })

  it('preserves a user-customised exposure across seed merges', async () => {
    // Pre-seed localStorage with a user value at seedVersion 0; the
    // load-time merge must NOT overwrite the user's choice.
    const seedKey = 'kmp:7c5e8a1f-4d2b-4e9a-b8c3-f1d2e3a4b5b1'
    window.localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        overrides: { [seedKey]: { exposure: 5.0 } },
        labels: { [seedKey]: 'CUSTOM LABEL' },
        seedVersion: 0,
      }),
    )
    vi.resetModules()
    const mod = await import('./materialOverrideStore')
    const after = mod.useMaterialOverrideStore.getState().getOverride(seedKey)
    expect(after).toEqual({ exposure: 5.0 })
    // Label is preserved (existing wins) and seedVersion is bumped.
    const parsed = JSON.parse(window.localStorage.getItem(STORAGE_KEY)!) as {
      labels: Record<string, string>
      seedVersion: number
    }
    expect(parsed.labels[seedKey]).toBe('CUSTOM LABEL')
    expect(parsed.seedVersion).toBe(1)
  })
})
