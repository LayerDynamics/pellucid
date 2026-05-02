/* @vitest-environment jsdom */
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { useRendererQueueStore, type QueueEntry } from './rendererQueueStore'
import * as plastiqClient from '@/lib/plastiq-client'

function entry(id: string, overrides: Partial<QueueEntry> = {}): QueueEntry {
  return {
    id,
    name: `Asset ${id}`,
    thumbnail_url: null,
    status: 'pending',
    ...overrides,
  }
}

describe('rendererQueueStore', () => {
  beforeEach(() => {
    useRendererQueueStore.getState().reset()
    vi.restoreAllMocks()
  })

  it('starts empty with currentIndex -1', () => {
    const s = useRendererQueueStore.getState()
    expect(s.entries).toEqual([])
    expect(s.currentIndex).toBe(-1)
    expect(s.current()).toBeNull()
  })

  it('setEntries seeds the queue and points at index 0', () => {
    const s = useRendererQueueStore.getState()
    s.setEntries([entry('a'), entry('b')])
    expect(useRendererQueueStore.getState().entries).toHaveLength(2)
    expect(useRendererQueueStore.getState().currentIndex).toBe(0)
    expect(useRendererQueueStore.getState().current()?.id).toBe('a')
  })

  it('advance walks forward and stops at the last entry', () => {
    const s = useRendererQueueStore.getState()
    s.setEntries([entry('a'), entry('b')])
    expect(s.advance()?.id).toBe('b')
    expect(s.advance()).toBeNull()
    expect(useRendererQueueStore.getState().currentIndex).toBe(1)
  })

  it('prev walks backward and stops at index 0', () => {
    const s = useRendererQueueStore.getState()
    s.setEntries([entry('a'), entry('b')])
    s.setCurrentIndex(1)
    expect(s.prev()?.id).toBe('a')
    expect(s.prev()).toBeNull()
    expect(useRendererQueueStore.getState().currentIndex).toBe(0)
  })

  it('skip is an alias for advance', () => {
    const s = useRendererQueueStore.getState()
    s.setEntries([entry('a'), entry('b'), entry('c')])
    expect(s.skip()?.id).toBe('b')
    expect(s.skip()?.id).toBe('c')
  })

  it('setStatus mutates only the named entry', () => {
    const s = useRendererQueueStore.getState()
    s.setEntries([entry('a'), entry('b')])
    s.setStatus('a', 'in_progress')
    expect(useRendererQueueStore.getState().entries[0].status).toBe('in_progress')
    expect(useRendererQueueStore.getState().entries[1].status).toBe('pending')
  })

  it('markCompleted removes the entry and keeps currentIndex valid', () => {
    const s = useRendererQueueStore.getState()
    s.setEntries([entry('a'), entry('b'), entry('c')])
    s.setCurrentIndex(1) // pointing at b

    s.markCompleted('a') // remove an earlier one
    let st = useRendererQueueStore.getState()
    expect(st.entries.map((e) => e.id)).toEqual(['b', 'c'])
    expect(st.currentIndex).toBe(0) // shifted down
    expect(st.current()?.id).toBe('b')

    s.markCompleted('b') // remove the current one
    st = useRendererQueueStore.getState()
    expect(st.entries.map((e) => e.id)).toEqual(['c'])
    expect(st.currentIndex).toBe(0)
    expect(st.current()?.id).toBe('c')

    s.markCompleted('c') // remove the last one
    st = useRendererQueueStore.getState()
    expect(st.entries).toEqual([])
    expect(st.currentIndex).toBe(-1)
  })

  it('markCompleted is a no-op for an unknown id', () => {
    const s = useRendererQueueStore.getState()
    s.setEntries([entry('a')])
    s.markCompleted('ghost')
    expect(useRendererQueueStore.getState().entries).toHaveLength(1)
  })

  it('hydrate calls listAssets with needs_render=true and stores the result', async () => {
    const spy = vi.spyOn(plastiqClient, 'listAssets').mockResolvedValue({
      count: 2,
      page: 1,
      page_size: 200,
      results: [
        {
          id: 'x',
          name: 'X',
          slug: 'x',
          created_at: '',
          updated_at: '',
          file_count: 0,
          version_count: 0,
          thumbnail_url: null,
          needs_render: true,
          render_complete: false,
        },
        {
          id: 'y',
          name: 'Y',
          slug: 'y',
          created_at: '',
          updated_at: '',
          file_count: 0,
          version_count: 0,
          thumbnail_url: null,
          needs_render: true,
          render_complete: true,
        },
      ],
    })
    await useRendererQueueStore.getState().hydrate()
    expect(spy).toHaveBeenCalledWith({ needs_render: true, page_size: 200 })
    const st = useRendererQueueStore.getState()
    expect(st.entries.map((e) => [e.id, e.status])).toEqual([
      ['x', 'pending'],
      ['y', 'done'],
    ])
    expect(st.currentIndex).toBe(0)
    expect(st.loading).toBe(false)
    expect(st.error).toBeNull()
  })

  it('hydrate captures error message on failure', async () => {
    vi.spyOn(plastiqClient, 'listAssets').mockRejectedValue(new Error('boom'))
    await useRendererQueueStore.getState().hydrate()
    const st = useRendererQueueStore.getState()
    expect(st.loading).toBe(false)
    expect(st.error).toBe('boom')
    expect(st.entries).toEqual([])
  })
})
