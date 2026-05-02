import { create } from 'zustand'

export interface AnimationClip {
  id: string
  name: string
  duration: number
  frameRate: number
}

export type PlaybackState = 'idle' | 'playing' | 'paused'

interface AnimationState {
  clips: AnimationClip[]
  activeClipId: string | null
  playback: PlaybackState
  currentTime: number
  loop: boolean
  speed: number

  setClips: (clips: AnimationClip[]) => void
  setActiveClip: (id: string | null) => void
  setPlayback: (state: PlaybackState) => void
  setCurrentTime: (time: number) => void
  setLoop: (loop: boolean) => void
  setSpeed: (speed: number) => void
  play: () => void
  pause: () => void
  stop: () => void
  reset: () => void
}

export const useAnimationStore = create<AnimationState>((set) => ({
  clips: [],
  activeClipId: null,
  playback: 'idle',
  currentTime: 0,
  loop: true,
  speed: 1,

  setClips: (clips) => set({ clips }),
  setActiveClip: (activeClipId) => set({ activeClipId, currentTime: 0, playback: 'idle' }),
  setPlayback: (playback) => set({ playback }),
  setCurrentTime: (currentTime) => set({ currentTime }),
  setLoop: (loop) => set({ loop }),
  setSpeed: (speed) => set({ speed }),
  play: () => set({ playback: 'playing' }),
  pause: () => set({ playback: 'paused' }),
  stop: () => set({ playback: 'idle', currentTime: 0 }),
  reset: () => set({ clips: [], activeClipId: null, playback: 'idle', currentTime: 0, loop: true, speed: 1 }),
}))
