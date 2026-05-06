/**
 * KeyboardShortcutsModal — static help dialog listing the global
 * keyboard surface. Single source of truth for the shortcuts so the
 * about-page and the help dropdown stay in sync.
 */

import { type ReactElement } from "react";

import { Dialog, DialogContent } from "../components/primitives/Dialog";
import { Button } from "../components/primitives/Button";
import { useUiStore } from "../state/useUiStore";

import { MODAL_SHORTCUTS } from "./index";

export interface ShortcutEntry {
  /** Stable id used as React key. */
  id: string;
  /** Keys to display, e.g. ["⌘", "K"]. */
  keys: string[];
  /** Human description of what the shortcut does. */
  description: string;
}

export const SHORTCUTS: ShortcutEntry[] = [
  { id: "search",      keys: ["⌘", "K"],         description: "Open command palette / search" },
  { id: "settings",    keys: ["⌘", ","],         description: "Open settings" },
  { id: "shortcuts",   keys: ["⌘", "/"],         description: "Show this shortcuts dialog" },
  { id: "tier",        keys: ["⌘", "U"],         description: "View / upgrade tier" },
  { id: "auth",        keys: ["⌘", "L"],         description: "Sign in or out" },
  { id: "sidebar",     keys: ["⌘", "B"],         description: "Toggle sidebar" },
  { id: "reload",      keys: ["⌘", "R"],         description: "Reload current panels" },
  { id: "close-modal", keys: ["Esc"],             description: "Close current modal" },
];

export function KeyboardShortcutsModal(): ReactElement {
  const isOpen = useUiStore((s) => s.isModalOpen(MODAL_SHORTCUTS));
  const popModal = useUiStore((s) => s.popModal);
  return (
    <Dialog open={isOpen} onOpenChange={(o) => { if (!o) popModal(); }}>
      <DialogContent
        data-modal-id={MODAL_SHORTCUTS}
        heading="Keyboard shortcuts"
        blurb="Global keyboard surface."
        className="w-[460px]"
      >
        <ul className="mt-3 flex flex-col gap-1.5" aria-label={`${SHORTCUTS.length} keyboard shortcuts`}>
          {SHORTCUTS.map((s) => (
            <li
              key={s.id}
              data-component="ShortcutRow"
              data-id={s.id}
              className="flex items-baseline justify-between gap-3 text-xs"
            >
              <span className="flex gap-1">
                {s.keys.map((k, i) => (
                  <kbd
                    key={`${s.id}-${i}`}
                    className="rounded border border-[var(--pellucid-border)] bg-[var(--pellucid-surface)] px-1.5 py-0.5 font-mono text-[10px]"
                  >
                    {k}
                  </kbd>
                ))}
              </span>
              <span className="flex-1 text-right text-[var(--pellucid-fg-muted)]">{s.description}</span>
            </li>
          ))}
        </ul>
        <div className="flex justify-end pt-3">
          <Button type="button" variant="solid" onClick={() => popModal()} data-field="close">
            Close
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
