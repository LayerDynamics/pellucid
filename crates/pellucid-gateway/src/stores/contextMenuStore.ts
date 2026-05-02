import { create } from 'zustand'

// SPEC-1 R-14, M8 — context menu store.
//
// One context menu open at a time; a new `open()` call replaces the prior
// menu. Position is in viewport pixels (clientX/clientY of the originating
// pointer event). Actions are an ordered list with optional separators and
// disabled flags. `targetId` is opaque to the store — callers (Renderer's
// scene picker, asset list right-click) use it to thread the click target
// through to the action handlers.
//
// Action handlers are invoked by id via `runAction`; the store does not
// auto-close on action — handlers may need to leave the menu open (e.g. a
// submenu pivot). `close()` is the explicit close.

export interface ContextMenuAction {
  id: string
  label: string
  /** Optional shortcut hint (e.g. "Ctrl+C"). */
  shortcut?: string
  /** Optional left-side icon — Radix-style ReactNode. */
  iconKey?: string
  disabled?: boolean
  destructive?: boolean
  /** Submenu actions (rendered as a flyout). */
  children?: ContextMenuAction[]
  run?(targetId: string | null): void | Promise<void>
}

export interface ContextMenuSeparator {
  separator: true
  id: string
}

export type ContextMenuItem = ContextMenuAction | ContextMenuSeparator

export function isSeparator(item: ContextMenuItem): item is ContextMenuSeparator {
  return (item as ContextMenuSeparator).separator === true
}

interface ContextMenuState {
  open: boolean
  position: { x: number; y: number } | null
  /** Opaque target identifier — caller-defined (asset id, mesh uuid, etc.). */
  targetId: string | null
  items: ContextMenuItem[]

  /** Open the menu at a position with the given items. */
  show: (input: {
    x: number
    y: number
    targetId?: string | null
    items: ContextMenuItem[]
  }) => void

  /** Close the menu (idempotent). */
  close: () => void

  /** Find an item by id (deep, including submenu children). */
  findAction(id: string): ContextMenuAction | null

  /** Invoke an action by id with the current targetId. Returns whether dispatched. */
  runAction(id: string): Promise<boolean>
}

function findActionDeep(items: ContextMenuItem[], id: string): ContextMenuAction | null {
  for (const item of items) {
    if (isSeparator(item)) continue
    if (item.id === id) return item
    if (item.children && item.children.length > 0) {
      const childItems: ContextMenuItem[] = item.children
      const hit = findActionDeep(childItems, id)
      if (hit) return hit
    }
  }
  return null
}

export const useContextMenuStore = create<ContextMenuState>((set, get) => ({
  open: false,
  position: null,
  targetId: null,
  items: [],

  show: ({ x, y, targetId = null, items }) => {
    set({ open: true, position: { x, y }, targetId, items })
  },

  close: () => {
    set({ open: false, position: null, targetId: null, items: [] })
  },

  findAction(id) {
    return findActionDeep(get().items, id)
  },

  async runAction(id) {
    const { items, targetId } = get()
    const action = findActionDeep(items, id)
    if (!action || action.disabled || !action.run) return false
    await action.run(targetId)
    return true
  },
}))
