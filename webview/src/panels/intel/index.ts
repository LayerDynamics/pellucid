/**
 * Intel panel-family registry.
 *
 * Mirrors `panels/news/index.ts` (T4.1.0) — same descriptor
 * shape, same `registerPanel` / `listPanels` / `findPanel` /
 * `panelsForVariant` / `panelsAvailableForTier` surface, but
 * scoped to its own registry array so the two families can be
 * iterated independently (the catalogue page renders one
 * section per family).
 *
 * The `news` family ships the shared sub-components every intel
 * panel reuses (`IntelEntityChip`, `SignalSeverityBadge`,
 * `BreakingTicker`, `NewsCard`); the intel registry just
 * re-exports them so consumers import from `@/panels/intel`
 * without reaching into `@/panels/news`.
 */

import { type ComponentType } from "react";

import {
  BreakingTicker,
  IntelEntityChip,
  NewsCard,
  SignalSeverityBadge,
  compareSeverity,
  type IntelEntity,
  type IntelEntityKind,
  type SignalSeverity,
} from "../news";

// Shared sub-components — re-exported so consumers import from
// "@/panels/intel" rather than reaching into the news family.
export {
  BreakingTicker,
  IntelEntityChip,
  NewsCard,
  SignalSeverityBadge,
  compareSeverity,
};
export type { IntelEntity, IntelEntityKind, SignalSeverity };

/** The 5 hosted SPA variants — same set as the news registry. */
export type Variant =
  | "base"
  | "tech"
  | "finance"
  | "commodity"
  | "happy";

/** Tier gate. */
export type Tier = 0 | 1 | 2 | 3;

/** One panel in the family. Same shape as the news family
 *  descriptor so a future cross-family catalogue can iterate
 *  both without re-typing the rows. */
export interface PanelDescriptor {
  /** Stable id — `<family>/<resource>`. */
  id: string;
  /** Display name. */
  title: string;
  /** Catalogue blurb. */
  blurb: string;
  /** React component. */
  component: ComponentType<unknown>;
  /** Cache keys the panel hydrates from on cold-start. */
  cacheKeys: string[];
  /** Minimum tier required to render. */
  minTier: Tier;
  /** Variants this panel is enabled in. `*` = all variants. */
  variants: Variant[] | "*";
}

const REGISTRY: PanelDescriptor[] = [];
const REGISTERED_IDS = new Set<string>();

/**
 * Register a panel descriptor. Each panel file calls this once
 * at module top-level. Throws on duplicate id — silent overwrite
 * would mask a real bug (two T4.1.x tasks racing the same id) and
 * break layout persistence in `usePanelStore`.
 *
 * @throws Error when `id` is already registered.
 */
export function registerPanel(descriptor: PanelDescriptor): void {
  if (REGISTERED_IDS.has(descriptor.id)) {
    throw new Error(
      `panels/intel: duplicate panel id ${JSON.stringify(descriptor.id)}`,
    );
  }
  REGISTERED_IDS.add(descriptor.id);
  REGISTRY.push(descriptor);
}

/**
 * Read the family's currently-registered panels. Returns a fresh
 * array snapshot.
 */
export function listPanels(): PanelDescriptor[] {
  return [...REGISTRY];
}

/**
 * Lookup helper. Returns `undefined` when the id isn't registered.
 */
export function findPanel(id: string): PanelDescriptor | undefined {
  return REGISTRY.find((p) => p.id === id);
}

/**
 * Filter the family by variant.
 */
export function panelsForVariant(variant: Variant): PanelDescriptor[] {
  return REGISTRY.filter(
    (p) => p.variants === "*" || p.variants.includes(variant),
  );
}

/**
 * Filter the family by minimum tier.
 */
export function panelsAvailableForTier(tier: Tier): PanelDescriptor[] {
  return REGISTRY.filter((p) => p.minTier <= tier);
}

/**
 * **Test-only.** Drop every registered panel.
 */
export function _resetRegistryForTests(): void {
  REGISTRY.length = 0;
  REGISTERED_IDS.clear();
}
