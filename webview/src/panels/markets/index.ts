/**
 * Markets / finance panel-family registry.
 *
 * Mirrors `panels/news/index.ts` (T4.1.0) — same descriptor
 * shape, same `registerPanel` / `listPanels` / `findPanel` /
 * `panelsForVariant` / `panelsAvailableForTier` surface, scoped
 * to its own registry array so a catalogue page can iterate
 * each family independently.
 *
 * The family ships four shared sub-components every market
 * panel reuses (`OhlcChart`, `MetricGrid`, `SymbolPicker`,
 * `WatchlistRow`); they are re-exported from this index so
 * consumers import from `@/panels/markets` rather than reaching
 * into individual files.
 */

import { type ComponentType } from "react";

import { OhlcChart } from "./OhlcChart";
import { MetricGrid } from "./MetricGrid";
import { SymbolPicker } from "./SymbolPicker";
import { WatchlistRow } from "./WatchlistRow";

// Shared sub-components — re-exported so consumers import from
// "@/panels/markets" rather than reaching into individual files.
export { OhlcChart, MetricGrid, SymbolPicker, WatchlistRow };
export type { OhlcCandle, OhlcChartProps } from "./OhlcChart";
export type { MetricGridProps, MetricTile } from "./MetricGrid";
export type { SymbolPickerProps } from "./SymbolPicker";
export type { WatchlistRowProps, WatchlistEntry } from "./WatchlistRow";

/** The 5 hosted SPA variants — same set as the news + intel
 *  registries. */
export type Variant =
  | "base"
  | "tech"
  | "finance"
  | "commodity"
  | "happy";

/** Tier gate. */
export type Tier = 0 | 1 | 2 | 3;

/** One panel in the family. Same shape as the news + intel
 *  family descriptors. */
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
 * would mask a real bug.
 *
 * @throws Error when `id` is already registered.
 */
export function registerPanel(descriptor: PanelDescriptor): void {
  if (REGISTERED_IDS.has(descriptor.id)) {
    throw new Error(
      `panels/markets: duplicate panel id ${JSON.stringify(descriptor.id)}`,
    );
  }
  REGISTERED_IDS.add(descriptor.id);
  REGISTRY.push(descriptor);
}

/** Read the family's currently-registered panels. */
export function listPanels(): PanelDescriptor[] {
  return [...REGISTRY];
}

/** Lookup helper. */
export function findPanel(id: string): PanelDescriptor | undefined {
  return REGISTRY.find((p) => p.id === id);
}

/** Filter the family by variant. */
export function panelsForVariant(variant: Variant): PanelDescriptor[] {
  return REGISTRY.filter(
    (p) => p.variants === "*" || p.variants.includes(variant),
  );
}

/** Filter the family by minimum tier. */
export function panelsAvailableForTier(tier: Tier): PanelDescriptor[] {
  return REGISTRY.filter((p) => p.minTier <= tier);
}

/** **Test-only.** Drop every registered panel. */
export function _resetRegistryForTests(): void {
  REGISTRY.length = 0;
  REGISTERED_IDS.clear();
}
