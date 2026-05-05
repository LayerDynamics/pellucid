/**
 * News / intel panel-family registry.
 *
 * Per `docs/plans/2026-04-25-pellucid-rebuild.md` §4.1.0, every
 * family ships a registry index that:
 *
 *  1. Re-exports the family's shared sub-components (`NewsCard`,
 *     `IntelEntityChip`, `BreakingTicker`, `SignalSeverityBadge`)
 *     so panels in this family — and the family showcase page —
 *     import from one stable surface.
 *  2. Declares the **panel descriptor list** for this family. A
 *     descriptor pairs a stable `id` (used by `usePanelStore`
 *     for layout persistence) with the React component, the
 *     cache keys it consumes, the minimum tier gate, and the
 *     variants the panel is enabled in.
 *
 * The descriptor list is **append-only**: T4.1.0 (this file)
 * lands the registry shape + the 4 shared sub-components; each
 * of T4.1.1–T4.1.8 adds its panel's descriptor by calling
 * [`registerPanel`] at the bottom of the panel's source file.
 * That keeps the registry forward-references to one place per
 * panel and makes the family's contents diff-friendly when a
 * panel is added or removed.
 */

import { type ComponentType } from "react";

import { NewsCard } from "./NewsCard";
import { IntelEntityChip } from "./IntelEntityChip";
import { BreakingTicker } from "./BreakingTicker";
import {
  SignalSeverityBadge,
  compareSeverity,
  type SignalSeverity,
} from "./SignalSeverityBadge";

// Shared sub-components — re-exported so consumers import from
// "@/panels/news" rather than reaching into individual files.
export {
  NewsCard,
  IntelEntityChip,
  BreakingTicker,
  SignalSeverityBadge,
  compareSeverity,
};
export type { SignalSeverity };
export type { NewsCardItem, NewsCardProps } from "./NewsCard";
export type {
  IntelEntity,
  IntelEntityKind,
  IntelEntityChipProps,
} from "./IntelEntityChip";
export type {
  BreakingTickerHeadline,
  BreakingTickerProps,
} from "./BreakingTicker";
export type { SignalSeverityBadgeProps } from "./SignalSeverityBadge";

/** The 5 hosted SPA variants. Mirrors `webview/src/config/variants/`
 *  so a panel descriptor can declare which variants list it. */
export type Variant =
  | "base"
  | "tech"
  | "finance"
  | "commodity"
  | "happy";

/** Tier gate enforced by the entitlement layer (`pellucid-auth`).
 *  Mirrors the `tier: 0|1|2|3` field on `useAuthStore`. */
export type Tier = 0 | 1 | 2 | 3;

/** One panel in the family. The catalogue / dashboard composer
 *  reads this; the rendered panel imports its descriptor's
 *  `component` so the registry holds the only forward reference. */
export interface PanelDescriptor {
  /** Stable id — also used as the `usePanelStore` layout key.
   *  Format: `<family>/<resource>` to match the original
   *  WorldMonitor convention so layout migrations carry over. */
  id: string;
  /** Display name for the catalogue + dashboard chrome. */
  title: string;
  /** Short one-liner shown beneath the title in the catalogue. */
  blurb: string;
  /** React component. T4.1.x panels register their component
   *  via [`registerPanel`] so the family can be statically
   *  iterated without dragging every component bundle in. */
  component: ComponentType<unknown>;
  /** Cache keys the panel hydrates from on cold-start. The
   *  bootstrap handler reads these before the panel mounts. */
  cacheKeys: string[];
  /** Minimum tier required to render. `0` = anonymous. */
  minTier: Tier;
  /** Variants this panel is enabled in. `*` = all variants. */
  variants: Variant[] | "*";
}

// Internal registry storage. Module-level mutable state is the
// intended pattern here — each panel module calls
// `registerPanel(...)` at top-level on import, mirroring how
// React Router's route registry works.
const REGISTRY: PanelDescriptor[] = [];
const REGISTERED_IDS = new Set<string>();

/**
 * Register a panel descriptor. Each panel file calls this once
 * at module top-level, e.g.:
 *
 * ```ts
 * import { registerPanel } from "./index";
 * export function NewsPanel(...) { ... }
 * registerPanel({ id: "news/feed", title: "News", ... });
 * ```
 *
 * Throws if a panel with the same `id` is already registered —
 * silent overwrite would mask a real bug (two T4.x tasks racing
 * the same id) and break layout persistence in `usePanelStore`.
 *
 * @throws Error when `id` is already registered.
 */
export function registerPanel(descriptor: PanelDescriptor): void {
  if (REGISTERED_IDS.has(descriptor.id)) {
    throw new Error(
      `panels/news: duplicate panel id ${JSON.stringify(descriptor.id)}`,
    );
  }
  REGISTERED_IDS.add(descriptor.id);
  REGISTRY.push(descriptor);
}

/**
 * Read the family's currently-registered panels. Returns a fresh
 * array snapshot so callers can safely sort / filter without
 * mutating the registry.
 */
export function listPanels(): PanelDescriptor[] {
  return [...REGISTRY];
}

/**
 * Lookup helper. Returns `undefined` when the id isn't registered
 * — callers should branch on this rather than assume presence
 * (the layout system stores stale ids until the user prunes them).
 */
export function findPanel(id: string): PanelDescriptor | undefined {
  return REGISTRY.find((p) => p.id === id);
}

/**
 * Filter the family by variant. The catalogue page calls this on
 * route entry so users see only panels they can actually open.
 */
export function panelsForVariant(variant: Variant): PanelDescriptor[] {
  return REGISTRY.filter(
    (p) => p.variants === "*" || p.variants.includes(variant),
  );
}

/**
 * Filter the family by minimum tier. Used by the catalogue's
 * "show locked panels" toggle and by the entitlement gate when
 * deciding whether to render an upgrade prompt vs hide the row.
 */
export function panelsAvailableForTier(tier: Tier): PanelDescriptor[] {
  return REGISTRY.filter((p) => p.minTier <= tier);
}

/**
 * **Test-only.** Drop every registered panel. The unit + family
 * integration tests register fixture panels and need a clean
 * slate between cases. Production code never calls this.
 */
export function _resetRegistryForTests(): void {
  REGISTRY.length = 0;
  REGISTERED_IDS.clear();
}
