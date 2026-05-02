import { create } from 'zustand'
import type {
  BackgroundMode,
  DirectionalLightConfig,
  ToneMappingMode,
  ShadowMapMode,
} from './rendererStore'

// ── Per-material environment overrides ──────────────────────────────────────
//
// This store holds a named set of environment-setting overrides for each
// material the operator may select in the renderer sidebar. When a material
// with overrides is picked, those fields on `useRendererStore` are
// overwritten with the override values. When the operator switches to a
// different material (or clears a field), each field that was previously
// overridden restores to the value it held immediately BEFORE the override
// kicked in — i.e. "exactly how the environment is now, but without any
// settings applied."
//
// Material key conventions (kept in sync with MaterialSelector rows):
//   • 'builtin:plastic'   — the Plastic Material row
//   • 'builtin:toon'      — the Toon Material row
//   • 'kmp:<entryId>'     — any KMP library entry (bundled or user-imported)
//
// Persistence: overrides are saved to localStorage under the key
// `plastiq.materialOverrides.v1`. The baseline snapshot (`savedBaseline`) and
// the active material key (`activeOverrideKey`) are session state and are
// intentionally NOT persisted — on page reload the renderer store starts
// from its own defaults, and the apply engine will re-capture a baseline the
// next time a material is selected.

// Every field the operator can override. Field names mirror the keys on
// `useRendererStore` 1:1 where possible; nested DirectionalLightConfig
// fields are flattened (`keyLightIntensity`, `keyLightAngleDeg`, …) so the
// apply engine can write them via `setKeyLight({ intensity })` etc.
export interface MaterialOverride {
  // Tone / exposure / output
  toneMapping?: ToneMappingMode
  exposure?: number
  shadowMapType?: ShadowMapMode

  // HDRI / environment
  hdriPresetId?: string
  hdriRotationDeg?: number
  hdriBlur?: number
  hdriContrast?: number
  hdriIntensity?: number
  backgroundIntensity?: number
  backgroundMode?: BackgroundMode
  backgroundColor?: string

  // Post-processing
  bloomEnabled?: boolean
  bloomStrength?: number
  bloomRadius?: number
  bloomThreshold?: number
  ssaoEnabled?: boolean
  ssaoIntensity?: number
  ssaoRadius?: number
  vignetteEnabled?: boolean
  vignetteOffset?: number
  vignetteDarkness?: number
  chromaticAberrationEnabled?: boolean
  chromaticAberrationOffset?: number

  // Key light (flattened from DirectionalLightConfig)
  keyLightIntensity?: number
  keyLightColor?: string
  keyLightAngleDeg?: number
  keyLightElevationDeg?: number

  // Fill light
  fillLightIntensity?: number
  fillLightColor?: string
  fillLightAngleDeg?: number
  fillLightElevationDeg?: number

  // Back light
  backLightIntensity?: number
  backLightColor?: string
  backLightAngleDeg?: number
  backLightElevationDeg?: number

  // Misc top-level toggles
  ambientIntensity?: number
  directionalIntensity?: number
  showGrid?: boolean
  showAxes?: boolean
  autoRotate?: boolean
  autoRotateSpeed?: number
}

export type MaterialOverrideField = keyof MaterialOverride

// The full enumerated field list. Used by the modal to iterate form rows
// and by the apply engine to know which renderer-store keys map to
// override fields (they are 1:1 by name except for the flattened light
// fields, which the apply engine joins back into DirectionalLightConfig
// patches).
export const MATERIAL_OVERRIDE_FIELDS: MaterialOverrideField[] = [
  'toneMapping',
  'exposure',
  'shadowMapType',
  'hdriPresetId',
  'hdriRotationDeg',
  'hdriBlur',
  'hdriContrast',
  'hdriIntensity',
  'backgroundIntensity',
  'backgroundMode',
  'backgroundColor',
  'bloomEnabled',
  'bloomStrength',
  'bloomRadius',
  'bloomThreshold',
  'ssaoEnabled',
  'ssaoIntensity',
  'ssaoRadius',
  'vignetteEnabled',
  'vignetteOffset',
  'vignetteDarkness',
  'chromaticAberrationEnabled',
  'chromaticAberrationOffset',
  'keyLightIntensity',
  'keyLightColor',
  'keyLightAngleDeg',
  'keyLightElevationDeg',
  'fillLightIntensity',
  'fillLightColor',
  'fillLightAngleDeg',
  'fillLightElevationDeg',
  'backLightIntensity',
  'backLightColor',
  'backLightAngleDeg',
  'backLightElevationDeg',
  'ambientIntensity',
  'directionalIntensity',
  'showGrid',
  'showAxes',
  'autoRotate',
  'autoRotateSpeed',
]

// A record of the pre-override value for every field that is CURRENTLY
// overridden by the active material. Used to restore the environment
// "exactly how it was, but without any settings applied" when the operator
// switches away or clears a field.
type BaselineSnapshot = Partial<Record<MaterialOverrideField, unknown>>

interface MaterialOverrideState {
  // Persistent: materialKey → override record
  overrides: Record<string, MaterialOverride>
  // Persistent: human-readable labels keyed alongside overrides — so the
  // modal header doesn't have to re-look up the label across the sidebar.
  labels: Record<string, string>
  // Session-only: the material key whose overrides are currently applied to
  // the renderer store (null before any material is picked this session).
  activeOverrideKey: string | null
  // Session-only: for each field listed in the active override, the value
  // the renderer store held BEFORE the override was applied. On material
  // switch or field clear we restore from this.
  savedBaseline: BaselineSnapshot

  getOverride: (key: string) => MaterialOverride | null
  hasOverride: (key: string) => boolean
  hasField: (key: string, field: MaterialOverrideField) => boolean
  getOverrideLabel: (key: string) => string | null

  // Modal / row-menu API
  setOverrideField: <F extends MaterialOverrideField>(
    key: string,
    label: string,
    field: F,
    value: NonNullable<MaterialOverride[F]>,
  ) => void
  clearOverrideField: (key: string, field: MaterialOverrideField) => void
  clearOverride: (key: string) => void

  // Engine API — internal, called by an apply-override engine
  _setActiveKey: (key: string | null) => void
  _captureBaseline: (field: MaterialOverrideField, value: unknown) => void
  _consumeBaseline: (field: MaterialOverrideField) => unknown | undefined
  _clearBaseline: () => void
}

export const MATERIAL_OVERRIDE_STORAGE_KEY = 'plastiq.materialOverrides.v1'

// Light-helper to nest the flat `<light>Intensity / Color / AngleDeg /
// ElevationDeg` field set back into a `DirectionalLightConfig` patch.
// Exported so the apply engine (when added) doesn't have to reimplement
// the flattening convention.
export function buildDirectionalLightPatch(
  override: MaterialOverride,
  prefix: 'keyLight' | 'fillLight' | 'backLight',
): Partial<DirectionalLightConfig> {
  const patch: Partial<DirectionalLightConfig> = {}
  const intensity = override[`${prefix}Intensity` as MaterialOverrideField]
  const color = override[`${prefix}Color` as MaterialOverrideField]
  const angleDeg = override[`${prefix}AngleDeg` as MaterialOverrideField]
  const elevationDeg = override[`${prefix}ElevationDeg` as MaterialOverrideField]
  if (typeof intensity === 'number') patch.intensity = intensity
  if (typeof color === 'string') patch.color = color
  if (typeof angleDeg === 'number') patch.angleDeg = angleDeg
  if (typeof elevationDeg === 'number') patch.elevationDeg = elevationDeg
  return patch
}

// Seeded defaults — applied to the persisted override store when the
// stored seedVersion is below the current SEED_VERSION. Field-level
// non-destructive merge: for each (materialKey, field) pair in
// SEEDED_OVERRIDES, the seed is written ONLY if the persisted override
// record for that material does not already carry that field. A user's
// explicit choice is always preserved — seeds only fill gaps. Once a
// material has its seeded fields merged in, the seedVersion counter is
// bumped and the merge is a no-op on subsequent loads.
const SEED_VERSION = 1

interface SeededEntry {
  /** Stable material key (e.g. `kmp:<uuid>` or `builtin:plastic`). */
  key: string
  /** Human-readable label paired with the override for the modal header. */
  label: string
  /** Partial override to merge non-destructively. */
  override: MaterialOverride
}

// PLASTIQ STANDARD BLACK is the canonical dark KMP preset and its baked
// `#1c1c1c` albedo reads under-lit against the default studio HDRI +
// exposure 1.0. Seeding exposure 2.5 lifts the silhouette to match the
// reference thumbnail's presence. The UUID is the deterministic id from
// `Loaders/Material.ts BUNDLED_SOURCES`. Users who prefer a different
// level override via the modal; their value persists through future
// SEED_VERSION bumps.
const PLASTIQ_STANDARD_BLACK_ID = '7c5e8a1f-4d2b-4e9a-b8c3-f1d2e3a4b5b1'

function buildSeededEntries(): SeededEntry[] {
  return [
    {
      key: `kmp:${PLASTIQ_STANDARD_BLACK_ID}`,
      label: 'PLASTIQ STANDARD BLACK',
      override: { exposure: 2.5 },
    },
  ]
}

interface StoredShape {
  overrides: Record<string, MaterialOverride>
  labels: Record<string, string>
  seedVersion: number
}

function readRawFromStorage(): Partial<StoredShape> {
  if (typeof window === 'undefined') return {}
  try {
    const raw = window.localStorage.getItem(MATERIAL_OVERRIDE_STORAGE_KEY)
    if (!raw) return {}
    return JSON.parse(raw) as Partial<StoredShape>
  } catch {
    return {}
  }
}

/** Merge seeded defaults into a persisted override map, field-by-field.
 *  Never overwrites an already-set field — user choices always win. Returns
 *  a new object so the caller can compare identity to decide whether to
 *  re-persist. */
function mergeSeeds(
  overrides: Record<string, MaterialOverride>,
  labels: Record<string, string>,
  entries: SeededEntry[],
): {
  overrides: Record<string, MaterialOverride>
  labels: Record<string, string>
  changed: boolean
} {
  let changed = false
  const nextOverrides: Record<string, MaterialOverride> = { ...overrides }
  const nextLabels: Record<string, string> = { ...labels }
  for (const entry of entries) {
    const existing = nextOverrides[entry.key] ?? {}
    const merged: MaterialOverride = { ...existing }
    let touched = false
    for (const [field, value] of Object.entries(entry.override) as Array<
      [MaterialOverrideField, MaterialOverride[MaterialOverrideField]]
    >) {
      if (!Object.prototype.hasOwnProperty.call(existing, field)) {
        ;(merged as Record<string, unknown>)[field] = value
        touched = true
      }
    }
    if (touched) {
      nextOverrides[entry.key] = merged
      if (!nextLabels[entry.key]) nextLabels[entry.key] = entry.label
      changed = true
    }
  }
  return { overrides: nextOverrides, labels: nextLabels, changed }
}

function loadPersisted(): StoredShape {
  if (typeof window === 'undefined') {
    return { overrides: {}, labels: {}, seedVersion: SEED_VERSION }
  }
  const parsed = readRawFromStorage()
  const storedOverrides =
    parsed.overrides && typeof parsed.overrides === 'object' ? parsed.overrides : {}
  const storedLabels =
    parsed.labels && typeof parsed.labels === 'object' ? parsed.labels : {}
  const storedSeedVersion =
    typeof parsed.seedVersion === 'number' ? parsed.seedVersion : 0

  if (storedSeedVersion < SEED_VERSION) {
    const seeded = mergeSeeds(storedOverrides, storedLabels, buildSeededEntries())
    const result: StoredShape = {
      overrides: seeded.overrides,
      labels: seeded.labels,
      seedVersion: SEED_VERSION,
    }
    if (seeded.changed || storedSeedVersion !== SEED_VERSION) {
      savePersisted(result)
    }
    return result
  }

  return {
    overrides: storedOverrides,
    labels: storedLabels,
    seedVersion: storedSeedVersion,
  }
}

function savePersisted(state: StoredShape): void {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(
      MATERIAL_OVERRIDE_STORAGE_KEY,
      JSON.stringify({
        overrides: state.overrides,
        labels: state.labels,
        seedVersion: state.seedVersion,
      }),
    )
  } catch {
    // localStorage is best-effort — quota errors, private mode, etc. The
    // live session state still works without persistence.
  }
}

const initial = loadPersisted()

export const useMaterialOverrideStore = create<MaterialOverrideState>((set, get) => ({
  overrides: initial.overrides,
  labels: initial.labels,
  activeOverrideKey: null,
  savedBaseline: {},

  getOverride: (key) => get().overrides[key] ?? null,
  hasOverride: (key) => {
    const o = get().overrides[key]
    return !!o && Object.keys(o).length > 0
  },
  hasField: (key, field) => {
    const o = get().overrides[key]
    return !!o && Object.prototype.hasOwnProperty.call(o, field)
  },
  getOverrideLabel: (key) => get().labels[key] ?? null,

  setOverrideField: (key, label, field, value) => {
    set((state) => {
      const existing = state.overrides[key] ?? {}
      const nextOverride = { ...existing, [field]: value } as MaterialOverride
      const nextOverrides = { ...state.overrides, [key]: nextOverride }
      const nextLabels = { ...state.labels, [key]: label }
      savePersisted({
        overrides: nextOverrides,
        labels: nextLabels,
        seedVersion: SEED_VERSION,
      })
      return { overrides: nextOverrides, labels: nextLabels }
    })
  },

  clearOverrideField: (key, field) => {
    set((state) => {
      const existing = state.overrides[key]
      if (!existing || !Object.prototype.hasOwnProperty.call(existing, field)) {
        return {}
      }
      const next = { ...existing } as MaterialOverride
      delete next[field]
      const nextOverrides = { ...state.overrides }
      const nextLabels = { ...state.labels }
      if (Object.keys(next).length === 0) {
        delete nextOverrides[key]
        delete nextLabels[key]
      } else {
        nextOverrides[key] = next
      }
      savePersisted({
        overrides: nextOverrides,
        labels: nextLabels,
        seedVersion: SEED_VERSION,
      })
      return { overrides: nextOverrides, labels: nextLabels }
    })
  },

  clearOverride: (key) => {
    set((state) => {
      if (!state.overrides[key]) return {}
      const nextOverrides = { ...state.overrides }
      delete nextOverrides[key]
      const nextLabels = { ...state.labels }
      delete nextLabels[key]
      savePersisted({
        overrides: nextOverrides,
        labels: nextLabels,
        seedVersion: SEED_VERSION,
      })
      return { overrides: nextOverrides, labels: nextLabels }
    })
  },

  _setActiveKey: (key) => set({ activeOverrideKey: key }),
  _captureBaseline: (field, value) =>
    set((state) => ({
      savedBaseline: { ...state.savedBaseline, [field]: value },
    })),
  _consumeBaseline: (field) => {
    const state = get()
    if (!Object.prototype.hasOwnProperty.call(state.savedBaseline, field)) {
      return undefined
    }
    const value = state.savedBaseline[field]
    const next = { ...state.savedBaseline }
    delete next[field]
    set({ savedBaseline: next })
    return value
  },
  _clearBaseline: () => set({ savedBaseline: {} }),
}))
