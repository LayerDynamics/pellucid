/* @vitest-environment jsdom */
import { describe, it, expect, beforeEach } from 'vitest'
import { useCameraOrientationStore } from './cameraOrientationStore'

describe('cameraOrientationStore', () => {
  beforeEach(() => {
    useCameraOrientationStore.getState().reset()
  })

  it('starts with current=null and empty history', () => {
    const s = useCameraOrientationStore.getState()
    expect(s.current).toBeNull()
    expect(s.canUndo()).toBe(false)
    expect(s.canRedo()).toBe(false)
  })

  it('push() sets current and resets the redo stack', () => {
    const s = useCameraOrientationStore.getState()
    s.push('front')
    expect(useCameraOrientationStore.getState().current).toBe('front')
    s.push('top')
    expect(useCameraOrientationStore.getState().current).toBe('top')
    expect(useCameraOrientationStore.getState().past).toEqual(['front'])
  })

  it('push() of the same direction is a no-op', () => {
    const s = useCameraOrientationStore.getState()
    s.push('front')
    s.push('front')
    expect(useCameraOrientationStore.getState().past).toEqual([])
  })

  it('undo() walks backward through history', () => {
    const s = useCameraOrientationStore.getState()
    s.push('front')
    s.push('top')
    s.push('iso-front-left')
    expect(s.undo()).toBe('top')
    expect(useCameraOrientationStore.getState().current).toBe('top')
    expect(s.undo()).toBe('front')
    expect(useCameraOrientationStore.getState().current).toBe('front')
    expect(s.undo()).toBeNull()
  })

  it('redo() walks forward through future stack', () => {
    const s = useCameraOrientationStore.getState()
    s.push('front')
    s.push('top')
    s.undo()
    expect(s.redo()).toBe('top')
    expect(useCameraOrientationStore.getState().current).toBe('top')
    expect(s.redo()).toBeNull()
  })

  it('push() after undo() clears the redo stack', () => {
    const s = useCameraOrientationStore.getState()
    s.push('front')
    s.push('top')
    s.undo() // back to front
    s.push('left') // diverged future
    expect(s.canRedo()).toBe(false)
    expect(useCameraOrientationStore.getState().future).toEqual([])
  })

  it('past is bounded by limit (FIFO eviction)', () => {
    const s = useCameraOrientationStore.getState()
    s.setLimit(2)
    s.push('front')
    s.push('top')
    s.push('left')
    // past holds last 2 entries before current; current is 'left'
    expect(useCameraOrientationStore.getState().past).toEqual(['front', 'top'])
    s.push('right')
    expect(useCameraOrientationStore.getState().past).toEqual(['top', 'left'])
  })

  it('canUndo / canRedo reflect the stacks', () => {
    const s = useCameraOrientationStore.getState()
    s.push('front')
    s.push('top')
    expect(s.canUndo()).toBe(true)
    expect(s.canRedo()).toBe(false)
    s.undo()
    expect(s.canUndo()).toBe(false)
    expect(s.canRedo()).toBe(true)
  })
})
