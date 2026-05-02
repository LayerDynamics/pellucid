import { create } from 'zustand'
import * as THREE from 'three'

// ── RendererStore ─────────────────────────────────────────────────────────────
//
// Central zustand slice for every live-tunable renderer setting read by
// the PlastiqEngine `Viewport/*` components (SPEC-2 §6). The store carries
// four conceptual groups:
//
//   1. Core render config (toneMapping, exposure, shadow map type, output
//      color space, pixelRatioMax) consumed by `Viewport/Renderer` and
//      `Viewport/Config`.
//   2. Scene lighting (key/fill/back light intensities, colors, angles,
//      elevations) consumed by `Viewport/Lighting`.
//   3. HDRI + background (preset id, contrast, intensity, background mode,
//      background color, blur, rotation) consumed by `Viewport/HDRI`.
//   4. Post-processing toggles + params (bloom, SSAO, vignette, chromatic
//      aberration) consumed by `Viewport/PostProcessing`.
//
// A separate `cameraRef` slot holds the active ViewportCamera's imperative
// handle so UI components outside the R3F tree (e.g. view-cube, preset
// buttons) can drive fitToObject / setViewDirection without prop-drilling.
// The handle shape is declared here to avoid a circular import with
// Viewport/Camera.

// ── Render Modes ─────────────────────────────────────────────────────────────

export type RenderMode = 'solid' | 'wireframe' | 'points' | 'normal'
export type CameraPreset = 'perspective' | 'top' | 'front' | 'right' | 'isometric'

// ── Tone Mapping & Shadow Map Enums ──────────────────────────────────────────
//
// String enums carried in the store, mapped to three.js constants at
// Viewport/Config application time. Strings keep the store JSON-serializable
// for persistence and debug inspection.

export type ToneMappingMode =
  | 'none'
  | 'linear'
  | 'reinhard'
  | 'cineon'
  | 'aces'
  | 'agx'
  | 'neutral'

export const TONE_MAPPING_MAP: Record<ToneMappingMode, THREE.ToneMapping> = {
  none: THREE.NoToneMapping,
  linear: THREE.LinearToneMapping,
  reinhard: THREE.ReinhardToneMapping,
  cineon: THREE.CineonToneMapping,
  aces: THREE.ACESFilmicToneMapping,
  agx: THREE.AgXToneMapping,
  neutral: THREE.NeutralToneMapping,
}

export type ShadowMapMode = 'basic' | 'pcf' | 'pcfsoft' | 'vsm'

export const SHADOW_MAP_TYPE_MAP: Record<ShadowMapMode, THREE.ShadowMapType> = {
  basic: THREE.BasicShadowMap,
  pcf: THREE.PCFShadowMap,
  pcfsoft: THREE.PCFSoftShadowMap,
  vsm: THREE.VSMShadowMap,
}

export type BackgroundMode = 'hdri' | 'color' | 'transparent'

// ── Lighting ─────────────────────────────────────────────────────────────────

export interface DirectionalLightConfig {
  intensity: number
  color: string
  /** Azimuth angle in degrees, 0 = +X, 90 = +Z (clockwise from above). */
  angleDeg: number
  /** Elevation angle in degrees, 0 = horizon, 90 = directly above. */
  elevationDeg: number
}

// ── Camera Imperative Handle ─────────────────────────────────────────────────
//
// Mirror of `CameraHandle` in Viewport/Camera.tsx. Declared here as a
// structural type to avoid a store → Viewport circular import.

export type CameraViewDirection =
  | 'front' | 'back' | 'left' | 'right' | 'top' | 'bottom'
  | 'iso-front-left' | 'iso-front-right'
  | 'iso-back-left' | 'iso-back-right'

export interface CameraHandleLike {
  fitToObject(obj: THREE.Object3D): void
  setViewDirection(dir: CameraViewDirection): void
}

// ── State ────────────────────────────────────────────────────────────────────

interface RendererState {
  // Legacy top-level toggles (retained for existing Controls)
  renderMode: RenderMode
  cameraPreset: CameraPreset
  showGrid: boolean
  showAxes: boolean
  backgroundColor: string
  autoRotate: boolean
  autoRotateSpeed: number
  ambientIntensity: number
  directionalIntensity: number

  // Core render config (SPEC-2 §6 / T7.13)
  toneMapping: ToneMappingMode
  exposure: number
  shadowMapType: ShadowMapMode
  outputColorSpace: THREE.ColorSpace
  pixelRatioMax: number

  // Lighting — three directional rig (SPEC-2 §8 M7)
  keyLight: DirectionalLightConfig
  fillLight: DirectionalLightConfig
  backLight: DirectionalLightConfig

  // HDRI + background (T7.12)
  hdriPresetId: string
  hdriContrast: number
  hdriIntensity: number
  hdriRotationDeg: number
  hdriBlur: number
  backgroundMode: BackgroundMode
  backgroundIntensity: number

  // Post-processing (T7.17)
  bloomEnabled: boolean
  bloomStrength: number
  bloomRadius: number
  bloomThreshold: number
  ssaoEnabled: boolean
  ssaoIntensity: number
  ssaoRadius: number
  vignetteEnabled: boolean
  vignetteOffset: number
  vignetteDarkness: number
  chromaticAberrationEnabled: boolean
  chromaticAberrationOffset: number

  // Active camera imperative handle (null until ViewportCamera mounts)
  cameraRef: CameraHandleLike | null

  // Setters (existing)
  setRenderMode: (mode: RenderMode) => void
  setCameraPreset: (preset: CameraPreset) => void
  setShowGrid: (show: boolean) => void
  setShowAxes: (show: boolean) => void
  setBackgroundColor: (color: string) => void
  setAutoRotate: (enabled: boolean) => void
  setAutoRotateSpeed: (speed: number) => void
  setAmbientIntensity: (intensity: number) => void
  setDirectionalIntensity: (intensity: number) => void

  // Setters (M7)
  setToneMapping: (mode: ToneMappingMode) => void
  setExposure: (exposure: number) => void
  setShadowMapType: (mode: ShadowMapMode) => void
  setOutputColorSpace: (space: THREE.ColorSpace) => void
  setPixelRatioMax: (ratio: number) => void

  setKeyLight: (patch: Partial<DirectionalLightConfig>) => void
  setFillLight: (patch: Partial<DirectionalLightConfig>) => void
  setBackLight: (patch: Partial<DirectionalLightConfig>) => void

  setHdriPresetId: (id: string) => void
  setHdriContrast: (c: number) => void
  setHdriIntensity: (i: number) => void
  setHdriRotationDeg: (deg: number) => void
  setHdriBlur: (b: number) => void
  setBackgroundMode: (mode: BackgroundMode) => void
  setBackgroundIntensity: (i: number) => void

  setBloomEnabled: (v: boolean) => void
  setBloomStrength: (v: number) => void
  setBloomRadius: (v: number) => void
  setBloomThreshold: (v: number) => void
  setSsaoEnabled: (v: boolean) => void
  setSsaoIntensity: (v: number) => void
  setSsaoRadius: (v: number) => void
  setVignetteEnabled: (v: boolean) => void
  setVignetteOffset: (v: number) => void
  setVignetteDarkness: (v: number) => void
  setChromaticAberrationEnabled: (v: boolean) => void
  setChromaticAberrationOffset: (v: number) => void

  setCameraRef: (ref: CameraHandleLike | null) => void

  reset: () => void
}

const defaults: Omit<RendererState,
  | 'setRenderMode' | 'setCameraPreset' | 'setShowGrid' | 'setShowAxes'
  | 'setBackgroundColor' | 'setAutoRotate' | 'setAutoRotateSpeed'
  | 'setAmbientIntensity' | 'setDirectionalIntensity'
  | 'setToneMapping' | 'setExposure' | 'setShadowMapType'
  | 'setOutputColorSpace' | 'setPixelRatioMax'
  | 'setKeyLight' | 'setFillLight' | 'setBackLight'
  | 'setHdriPresetId' | 'setHdriContrast' | 'setHdriIntensity'
  | 'setHdriRotationDeg' | 'setHdriBlur' | 'setBackgroundMode'
  | 'setBackgroundIntensity'
  | 'setBloomEnabled' | 'setBloomStrength' | 'setBloomRadius' | 'setBloomThreshold'
  | 'setSsaoEnabled' | 'setSsaoIntensity' | 'setSsaoRadius'
  | 'setVignetteEnabled' | 'setVignetteOffset' | 'setVignetteDarkness'
  | 'setChromaticAberrationEnabled' | 'setChromaticAberrationOffset'
  | 'setCameraRef'
  | 'reset'
> = {
  renderMode: 'solid',
  cameraPreset: 'perspective',
  showGrid: true,
  showAxes: false,
  backgroundColor: '#1a1a1a',
  autoRotate: false,
  autoRotateSpeed: 1,
  ambientIntensity: 0.6,
  directionalIntensity: 1.0,

  toneMapping: 'aces',
  exposure: 1.0,
  shadowMapType: 'pcfsoft',
  outputColorSpace: THREE.SRGBColorSpace,
  pixelRatioMax: 2,

  keyLight: {
    intensity: 1.0,
    color: '#ffffff',
    angleDeg: 45,
    elevationDeg: 45,
  },
  fillLight: {
    intensity: 0.4,
    color: '#ffffff',
    angleDeg: -60,
    elevationDeg: 30,
  },
  backLight: {
    intensity: 0.6,
    color: '#ffffff',
    angleDeg: 180,
    elevationDeg: 40,
  },

  hdriPresetId: 'studio',
  hdriContrast: 0,
  hdriIntensity: 1.0,
  hdriRotationDeg: 0,
  hdriBlur: 0,
  backgroundMode: 'color',
  backgroundIntensity: 1.0,

  bloomEnabled: false,
  bloomStrength: 0.5,
  bloomRadius: 0.4,
  bloomThreshold: 0.85,
  ssaoEnabled: false,
  ssaoIntensity: 0.5,
  ssaoRadius: 0.1,
  vignetteEnabled: false,
  vignetteOffset: 0.5,
  vignetteDarkness: 0.5,
  chromaticAberrationEnabled: false,
  chromaticAberrationOffset: 0.002,

  cameraRef: null,
}

export const useRendererStore = create<RendererState>((set) => ({
  ...defaults,

  setRenderMode: (renderMode) => set({ renderMode }),
  setCameraPreset: (cameraPreset) => set({ cameraPreset }),
  setShowGrid: (showGrid) => set({ showGrid }),
  setShowAxes: (showAxes) => set({ showAxes }),
  setBackgroundColor: (backgroundColor) => set({ backgroundColor }),
  setAutoRotate: (autoRotate) => set({ autoRotate }),
  setAutoRotateSpeed: (autoRotateSpeed) => set({ autoRotateSpeed }),
  setAmbientIntensity: (ambientIntensity) => set({ ambientIntensity }),
  setDirectionalIntensity: (directionalIntensity) => set({ directionalIntensity }),

  setToneMapping: (toneMapping) => set({ toneMapping }),
  setExposure: (exposure) => set({ exposure }),
  setShadowMapType: (shadowMapType) => set({ shadowMapType }),
  setOutputColorSpace: (outputColorSpace) => set({ outputColorSpace }),
  setPixelRatioMax: (pixelRatioMax) => set({ pixelRatioMax }),

  setKeyLight: (patch) =>
    set((s) => ({ keyLight: { ...s.keyLight, ...patch } })),
  setFillLight: (patch) =>
    set((s) => ({ fillLight: { ...s.fillLight, ...patch } })),
  setBackLight: (patch) =>
    set((s) => ({ backLight: { ...s.backLight, ...patch } })),

  setHdriPresetId: (hdriPresetId) => set({ hdriPresetId }),
  setHdriContrast: (hdriContrast) => set({ hdriContrast }),
  setHdriIntensity: (hdriIntensity) => set({ hdriIntensity }),
  setHdriRotationDeg: (hdriRotationDeg) => set({ hdriRotationDeg }),
  setHdriBlur: (hdriBlur) => set({ hdriBlur }),
  setBackgroundMode: (backgroundMode) => set({ backgroundMode }),
  setBackgroundIntensity: (backgroundIntensity) => set({ backgroundIntensity }),

  setBloomEnabled: (bloomEnabled) => set({ bloomEnabled }),
  setBloomStrength: (bloomStrength) => set({ bloomStrength }),
  setBloomRadius: (bloomRadius) => set({ bloomRadius }),
  setBloomThreshold: (bloomThreshold) => set({ bloomThreshold }),
  setSsaoEnabled: (ssaoEnabled) => set({ ssaoEnabled }),
  setSsaoIntensity: (ssaoIntensity) => set({ ssaoIntensity }),
  setSsaoRadius: (ssaoRadius) => set({ ssaoRadius }),
  setVignetteEnabled: (vignetteEnabled) => set({ vignetteEnabled }),
  setVignetteOffset: (vignetteOffset) => set({ vignetteOffset }),
  setVignetteDarkness: (vignetteDarkness) => set({ vignetteDarkness }),
  setChromaticAberrationEnabled: (chromaticAberrationEnabled) =>
    set({ chromaticAberrationEnabled }),
  setChromaticAberrationOffset: (chromaticAberrationOffset) =>
    set({ chromaticAberrationOffset }),

  setCameraRef: (cameraRef) => set({ cameraRef }),

  reset: () => set(defaults),
}))
