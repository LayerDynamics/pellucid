/**
 * SearchModal — cmd-K command palette. Browses every panel registered
 * across panels/{macro,energy,climate,infra,forecast,intel,news,markets,
 * aviation} via panelsAvailableForTier filtering and lets the user
 * select one to focus.
 *
 * Real fuzzy matching here — no fixture lists.
 */

import { useEffect, useMemo, useState, type ReactElement } from "react";

import { Dialog, DialogContent } from "../components/primitives/Dialog";
import { useUiStore } from "../state/useUiStore";
import { useAuthStore } from "../state/useAuthStore";
import { usePanelStore } from "../state/usePanelStore";

import { listPanels as listMacro } from "../panels/macro/index";
import { listPanels as listEnergy } from "../panels/energy/index";
import { listPanels as listClimate } from "../panels/climate/index";
import { listPanels as listInfra } from "../panels/infra/index";
import { listPanels as listForecast } from "../panels/forecast/index";

import { MODAL_SEARCH } from "./index";

export interface SearchEntry {
  id: string;
  family: string;
  title: string;
  blurb: string;
  minTier: number;
}

export function collectAllPanels(): SearchEntry[] {
  const families: { family: string; entries: ReturnType<typeof listMacro> }[] = [
    { family: "macro", entries: listMacro() },
    { family: "energy", entries: listEnergy() },
    { family: "climate", entries: listClimate() },
    { family: "infra", entries: listInfra() },
    { family: "forecast", entries: listForecast() },
  ];
  return families.flatMap((f) =>
    f.entries.map((e) => ({
      id: e.id,
      family: f.family,
      title: e.title,
      blurb: e.blurb,
      minTier: e.minTier,
    })),
  );
}

/**
 * Substring-and-token fuzzy match: query is lower-cased, split on
 * whitespace, and every token must appear somewhere in
 * `${title} ${blurb} ${family}`.
 */
export function matchEntries(entries: SearchEntry[], query: string): SearchEntry[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return entries;
  const tokens = q.split(/\s+/u);
  return entries.filter((e) => {
    const hay = `${e.title} ${e.blurb} ${e.family}`.toLowerCase();
    return tokens.every((t) => hay.includes(t));
  });
}

export function SearchModal(): ReactElement {
  const isOpen = useUiStore((s) => s.isModalOpen(MODAL_SEARCH));
  const popModal = useUiStore((s) => s.popModal);
  const tier = useAuthStore((s) => s.entitlements?.tier ?? 0);
  const setHighlighted = usePanelStore((s) => s.setHighlighted);
  const [query, setQuery] = useState("");
  const [activeIdx, setActiveIdx] = useState(0);

  useEffect(() => {
    if (!isOpen) {
      setQuery("");
      setActiveIdx(0);
    }
  }, [isOpen]);

  const entries = useMemo(() => collectAllPanels().filter((e) => e.minTier <= tier), [tier]);
  const filtered = useMemo(() => matchEntries(entries, query), [entries, query]);

  function commit(entry: SearchEntry): void {
    setHighlighted(entry.id);
    popModal();
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>): void {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIdx((i) => (filtered.length === 0 ? 0 : (i + 1) % filtered.length));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIdx((i) => (filtered.length === 0 ? 0 : (i - 1 + filtered.length) % filtered.length));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const target = filtered[activeIdx];
      if (target) commit(target);
    }
  }

  return (
    <Dialog open={isOpen} onOpenChange={(o) => { if (!o) popModal(); }}>
      <DialogContent
        data-modal-id={MODAL_SEARCH}
        heading="Search panels"
        blurb={`${filtered.length} of ${entries.length} match.`}
        className="w-[600px]"
      >
        <input
          type="text"
          value={query}
          onChange={(e) => { setQuery(e.target.value); setActiveIdx(0); }}
          onKeyDown={onKeyDown}
          placeholder="Type to filter…"
          autoFocus
          data-field="query"
          aria-label="Filter panels"
          className="mt-3 w-full rounded border border-[var(--pellucid-border)] bg-transparent px-2 py-1.5 text-sm focus:outline-none focus:border-[var(--pellucid-info)]"
        />
        <ul
          className="mt-2 max-h-[360px] overflow-y-auto"
          aria-label={`${filtered.length} matching panels`}
        >
          {filtered.length === 0 ? (
            <li data-state="empty" className="text-xs text-[var(--pellucid-fg-muted)] py-2">
              No matches.
            </li>
          ) : (
            filtered.map((entry, idx) => (
              <li
                key={entry.id}
                data-component="SearchResult"
                data-panel-id={entry.id}
                data-active={idx === activeIdx ? "true" : "false"}
                onMouseEnter={() => setActiveIdx(idx)}
                onClick={() => commit(entry)}
                className={`flex cursor-pointer flex-col rounded px-2 py-1.5 text-xs ${idx === activeIdx ? "bg-[var(--pellucid-surface)]" : ""}`}
              >
                <div className="flex items-baseline justify-between">
                  <span data-field="title" className="font-semibold">{entry.title}</span>
                  <span data-field="family" className="text-[10px] uppercase font-mono text-[var(--pellucid-fg-muted)]">{entry.family}</span>
                </div>
                <span data-field="blurb" className="text-[var(--pellucid-fg-muted)]">{entry.blurb}</span>
              </li>
            ))
          )}
        </ul>
      </DialogContent>
    </Dialog>
  );
}
