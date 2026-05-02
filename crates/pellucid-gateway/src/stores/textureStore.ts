import { create } from 'zustand'

export interface TextureSlot {
  id: string
  name: string
  mapType: 'albedo' | 'normal' | 'roughness' | 'metalness' | 'emissive' | 'ao' | 'height' | 'opacity'
  url: string | null
  tileU: number
  tileV: number
  offsetU: number
  offsetV: number
  rotation: number
}

interface TextureState {
  slots: TextureSlot[]
  activeSlotId: string | null

  setSlots: (slots: TextureSlot[]) => void
  addSlot: (slot: TextureSlot) => void
  updateSlot: (id: string, patch: Partial<TextureSlot>) => void
  removeSlot: (id: string) => void
  setActiveSlot: (id: string | null) => void
  assignTexture: (slotId: string, url: string) => void
  clearTexture: (slotId: string) => void
  reset: () => void
}

const defaultSlots: TextureSlot[] = [
  { id: 'albedo', name: 'Albedo', mapType: 'albedo', url: null, tileU: 1, tileV: 1, offsetU: 0, offsetV: 0, rotation: 0 },
  { id: 'normal', name: 'Normal', mapType: 'normal', url: null, tileU: 1, tileV: 1, offsetU: 0, offsetV: 0, rotation: 0 },
  { id: 'roughness', name: 'Roughness', mapType: 'roughness', url: null, tileU: 1, tileV: 1, offsetU: 0, offsetV: 0, rotation: 0 },
  { id: 'metalness', name: 'Metalness', mapType: 'metalness', url: null, tileU: 1, tileV: 1, offsetU: 0, offsetV: 0, rotation: 0 },
]

export const useTextureStore = create<TextureState>((set) => ({
  slots: defaultSlots,
  activeSlotId: 'albedo',

  setSlots: (slots) => set({ slots }),
  addSlot: (slot) => set((state) => ({ slots: [...state.slots, slot] })),
  updateSlot: (id, patch) =>
    set((state) => ({
      slots: state.slots.map((s) => (s.id === id ? { ...s, ...patch } : s)),
    })),
  removeSlot: (id) =>
    set((state) => ({ slots: state.slots.filter((s) => s.id !== id) })),
  setActiveSlot: (activeSlotId) => set({ activeSlotId }),
  assignTexture: (slotId, url) =>
    set((state) => ({
      slots: state.slots.map((s) => (s.id === slotId ? { ...s, url } : s)),
    })),
  clearTexture: (slotId) =>
    set((state) => ({
      slots: state.slots.map((s) => (s.id === slotId ? { ...s, url: null } : s)),
    })),
  reset: () => set({ slots: defaultSlots, activeSlotId: 'albedo' }),
}))
