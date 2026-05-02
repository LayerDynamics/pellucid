import { describe, it, expect, beforeEach } from 'vitest'
import * as THREE from 'three'
import {
  useRendererStore,
  TONE_MAPPING_MAP,
  SHADOW_MAP_TYPE_MAP,
  type ToneMappingMode,
  type ShadowMapMode,
  type BackgroundMode,
  type CameraHandleLike,
} from './rendererStore'

// Fresh snapshot helper — rendererStore is a singleton across tests, so
// each case must reset() before asserting on defaults to prevent state
// leak when the file is run as part of the whole test suite.
function resetStore(): void {
  useRendererStore.getState().reset()
}

describe('rendererStore', () => {
  beforeEach(resetStore)

  describe('TONE_MAPPING_MAP', () => {
    it('maps every ToneMappingMode key to a THREE.ToneMapping constant', () => {
      const modes: ToneMappingMode[] = [
        'none', 'linear', 'reinhard', 'cineon', 'aces', 'agx', 'neutral',
      ]
      for (const mode of modes) {
        expect(TONE_MAPPING_MAP[mode]).toBeTypeOf('number')
      }
      // Check a distinct value per mode (no accidental aliases)
      expect(TONE_MAPPING_MAP.none).toBe(THREE.NoToneMapping)
      expect(TONE_MAPPING_MAP.aces).toBe(THREE.ACESFilmicToneMapping)
      expect(TONE_MAPPING_MAP.agx).toBe(THREE.AgXToneMapping)
      expect(TONE_MAPPING_MAP.neutral).toBe(THREE.NeutralToneMapping)
    })
  })

  describe('SHADOW_MAP_TYPE_MAP', () => {
    it('maps every ShadowMapMode key to a THREE.ShadowMapType', () => {
      const modes: ShadowMapMode[] = ['basic', 'pcf', 'pcfsoft', 'vsm']
      for (const m of modes) {
        expect(SHADOW_MAP_TYPE_MAP[m]).toBeTypeOf('number')
      }
      expect(SHADOW_MAP_TYPE_MAP.pcfsoft).toBe(THREE.PCFSoftShadowMap)
      expect(SHADOW_MAP_TYPE_MAP.vsm).toBe(THREE.VSMShadowMap)
    })
  })

  describe('defaults', () => {
    it('seeds sensible M7 defaults', () => {
      const s = useRendererStore.getState()
      expect(s.toneMapping).toBe('aces')
      expect(s.exposure).toBe(1.0)
      expect(s.shadowMapType).toBe('pcfsoft')
      expect(s.outputColorSpace).toBe(THREE.SRGBColorSpace)
      expect(s.pixelRatioMax).toBe(2)

      expect(s.keyLight.intensity).toBeGreaterThan(0)
      expect(s.fillLight.intensity).toBeGreaterThan(0)
      expect(s.backLight.intensity).toBeGreaterThan(0)

      expect(s.hdriPresetId).toBe('studio')
      expect(s.hdriContrast).toBe(0)
      expect(s.backgroundMode).toBe('color')

      expect(s.bloomEnabled).toBe(false)
      expect(s.ssaoEnabled).toBe(false)
      expect(s.vignetteEnabled).toBe(false)
      expect(s.chromaticAberrationEnabled).toBe(false)

      expect(s.cameraRef).toBeNull()
    })

    it('preserves legacy toggles for existing consumers', () => {
      const s = useRendererStore.getState()
      expect(s.renderMode).toBe('solid')
      expect(s.cameraPreset).toBe('perspective')
      expect(s.showGrid).toBe(true)
      expect(s.showAxes).toBe(false)
    })
  })

  describe('core render config setters', () => {
    it('setToneMapping updates the active mode', () => {
      useRendererStore.getState().setToneMapping('cineon')
      expect(useRendererStore.getState().toneMapping).toBe('cineon')
    })

    it('setExposure clamps nothing — caller is responsible for range', () => {
      useRendererStore.getState().setExposure(2.5)
      expect(useRendererStore.getState().exposure).toBe(2.5)
      useRendererStore.getState().setExposure(0)
      expect(useRendererStore.getState().exposure).toBe(0)
    })

    it('setShadowMapType + setOutputColorSpace + setPixelRatioMax work', () => {
      useRendererStore.getState().setShadowMapType('vsm')
      useRendererStore.getState().setOutputColorSpace(THREE.LinearSRGBColorSpace)
      useRendererStore.getState().setPixelRatioMax(1)
      const s = useRendererStore.getState()
      expect(s.shadowMapType).toBe('vsm')
      expect(s.outputColorSpace).toBe(THREE.LinearSRGBColorSpace)
      expect(s.pixelRatioMax).toBe(1)
    })
  })

  describe('directional light setters (partial patch)', () => {
    it('setKeyLight merges partial into existing config', () => {
      useRendererStore.getState().setKeyLight({ intensity: 2.0 })
      const s = useRendererStore.getState()
      expect(s.keyLight.intensity).toBe(2.0)
      // Unspecified fields remain at defaults
      expect(s.keyLight.color).toBe('#ffffff')
      expect(s.keyLight.angleDeg).toBe(45)
    })

    it('setFillLight updates only the patched keys', () => {
      useRendererStore.getState().setFillLight({ angleDeg: -90, elevationDeg: 60 })
      const s = useRendererStore.getState()
      expect(s.fillLight.angleDeg).toBe(-90)
      expect(s.fillLight.elevationDeg).toBe(60)
      expect(s.fillLight.intensity).toBe(0.4)
    })

    it('setBackLight works independently of key/fill', () => {
      useRendererStore.getState().setKeyLight({ intensity: 9 })
      useRendererStore.getState().setBackLight({ color: '#ff0000' })
      const s = useRendererStore.getState()
      expect(s.keyLight.intensity).toBe(9)
      expect(s.backLight.color).toBe('#ff0000')
      expect(s.fillLight.intensity).toBe(0.4)
    })
  })

  describe('HDRI setters', () => {
    it('updates preset id, contrast, intensity, rotation, blur', () => {
      const a = useRendererStore.getState()
      a.setHdriPresetId('outdoor')
      a.setHdriContrast(0.5)
      a.setHdriIntensity(1.5)
      a.setHdriRotationDeg(45)
      a.setHdriBlur(0.1)
      const s = useRendererStore.getState()
      expect(s.hdriPresetId).toBe('outdoor')
      expect(s.hdriContrast).toBe(0.5)
      expect(s.hdriIntensity).toBe(1.5)
      expect(s.hdriRotationDeg).toBe(45)
      expect(s.hdriBlur).toBe(0.1)
    })

    it('background mode + intensity route through dedicated setters', () => {
      const modes: BackgroundMode[] = ['hdri', 'color', 'transparent']
      for (const m of modes) {
        useRendererStore.getState().setBackgroundMode(m)
        expect(useRendererStore.getState().backgroundMode).toBe(m)
      }
      useRendererStore.getState().setBackgroundIntensity(0.3)
      expect(useRendererStore.getState().backgroundIntensity).toBe(0.3)
    })
  })

  describe('post-processing setters', () => {
    it('bloom toggles + params update independently', () => {
      const a = useRendererStore.getState()
      a.setBloomEnabled(true)
      a.setBloomStrength(0.9)
      a.setBloomRadius(0.6)
      a.setBloomThreshold(0.5)
      const s = useRendererStore.getState()
      expect(s.bloomEnabled).toBe(true)
      expect(s.bloomStrength).toBe(0.9)
      expect(s.bloomRadius).toBe(0.6)
      expect(s.bloomThreshold).toBe(0.5)
    })

    it('SSAO + vignette + chromatic aberration toggle + numeric setters', () => {
      const a = useRendererStore.getState()
      a.setSsaoEnabled(true)
      a.setSsaoIntensity(0.7)
      a.setSsaoRadius(0.2)
      a.setVignetteEnabled(true)
      a.setVignetteOffset(0.4)
      a.setVignetteDarkness(0.8)
      a.setChromaticAberrationEnabled(true)
      a.setChromaticAberrationOffset(0.004)
      const s = useRendererStore.getState()
      expect(s.ssaoEnabled).toBe(true)
      expect(s.ssaoIntensity).toBe(0.7)
      expect(s.ssaoRadius).toBe(0.2)
      expect(s.vignetteEnabled).toBe(true)
      expect(s.vignetteOffset).toBe(0.4)
      expect(s.vignetteDarkness).toBe(0.8)
      expect(s.chromaticAberrationEnabled).toBe(true)
      expect(s.chromaticAberrationOffset).toBe(0.004)
    })
  })

  describe('cameraRef slot', () => {
    it('starts null and accepts a CameraHandleLike', () => {
      expect(useRendererStore.getState().cameraRef).toBeNull()
      const handle: CameraHandleLike = {
        fitToObject: () => {},
        setViewDirection: () => {},
      }
      useRendererStore.getState().setCameraRef(handle)
      expect(useRendererStore.getState().cameraRef).toBe(handle)
      useRendererStore.getState().setCameraRef(null)
      expect(useRendererStore.getState().cameraRef).toBeNull()
    })
  })

  describe('reset()', () => {
    it('restores every M7 field to its default value', () => {
      const a = useRendererStore.getState()
      a.setToneMapping('cineon')
      a.setExposure(3)
      a.setKeyLight({ intensity: 9 })
      a.setBloomEnabled(true)
      a.setCameraRef({
        fitToObject: () => {},
        setViewDirection: () => {},
      })

      useRendererStore.getState().reset()

      const s = useRendererStore.getState()
      expect(s.toneMapping).toBe('aces')
      expect(s.exposure).toBe(1.0)
      expect(s.keyLight.intensity).toBe(1.0)
      expect(s.bloomEnabled).toBe(false)
      expect(s.cameraRef).toBeNull()
    })
  })
})
