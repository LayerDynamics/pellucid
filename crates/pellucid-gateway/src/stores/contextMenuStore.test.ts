/* @vitest-environment jsdom */
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { useContextMenuStore, isSeparator, type ContextMenuItem } from './contextMenuStore'

describe('contextMenuStore', () => {
  beforeEach(() => {
    useContextMenuStore.getState().close()
  })

  it('starts closed', () => {
    const s = useContextMenuStore.getState()
    expect(s.open).toBe(false)
    expect(s.position).toBeNull()
    expect(s.targetId).toBeNull()
    expect(s.items).toEqual([])
  })

  it('show() opens the menu with position, target, and items', () => {
    const s = useContextMenuStore.getState()
    const items: ContextMenuItem[] = [
      { id: 'a', label: 'Action A' },
      { separator: true, id: 'sep1' },
      { id: 'b', label: 'Action B', destructive: true },
    ]
    s.show({ x: 100, y: 200, targetId: 'asset-7', items })
    const st = useContextMenuStore.getState()
    expect(st.open).toBe(true)
    expect(st.position).toEqual({ x: 100, y: 200 })
    expect(st.targetId).toBe('asset-7')
    expect(st.items).toEqual(items)
  })

  it('isSeparator narrows the union correctly', () => {
    expect(isSeparator({ separator: true, id: 'x' })).toBe(true)
    expect(isSeparator({ id: 'a', label: 'A' })).toBe(false)
  })

  it('close() resets to initial state', () => {
    const s = useContextMenuStore.getState()
    s.show({ x: 1, y: 2, items: [{ id: 'a', label: 'A' }] })
    s.close()
    const st = useContextMenuStore.getState()
    expect(st.open).toBe(false)
    expect(st.position).toBeNull()
    expect(st.items).toEqual([])
  })

  it('findAction locates top-level entries', () => {
    const s = useContextMenuStore.getState()
    s.show({
      x: 0,
      y: 0,
      items: [
        { id: 'a', label: 'A' },
        { id: 'b', label: 'B' },
      ],
    })
    expect(s.findAction('a')?.label).toBe('A')
    expect(s.findAction('missing')).toBeNull()
  })

  it('findAction recurses into submenu children', () => {
    const s = useContextMenuStore.getState()
    s.show({
      x: 0,
      y: 0,
      items: [
        {
          id: 'parent',
          label: 'Parent',
          children: [
            { id: 'child', label: 'Child' },
            { id: 'grand-parent', label: 'Grand', children: [{ id: 'leaf', label: 'Leaf' }] },
          ],
        },
      ],
    })
    expect(s.findAction('child')?.label).toBe('Child')
    expect(s.findAction('leaf')?.label).toBe('Leaf')
  })

  it('runAction invokes the handler with current targetId and returns true', async () => {
    const handler = vi.fn()
    const s = useContextMenuStore.getState()
    s.show({
      x: 0,
      y: 0,
      targetId: 'asset-7',
      items: [{ id: 'fire', label: 'Fire', run: handler }],
    })
    const ok = await s.runAction('fire')
    expect(ok).toBe(true)
    expect(handler).toHaveBeenCalledWith('asset-7')
  })

  it('runAction returns false for disabled actions', async () => {
    const handler = vi.fn()
    const s = useContextMenuStore.getState()
    s.show({
      x: 0,
      y: 0,
      items: [{ id: 'fire', label: 'Fire', disabled: true, run: handler }],
    })
    const ok = await s.runAction('fire')
    expect(ok).toBe(false)
    expect(handler).not.toHaveBeenCalled()
  })

  it('runAction returns false for unknown id', async () => {
    const s = useContextMenuStore.getState()
    s.show({ x: 0, y: 0, items: [{ id: 'a', label: 'A', run: () => {} }] })
    const ok = await s.runAction('ghost')
    expect(ok).toBe(false)
  })
})
