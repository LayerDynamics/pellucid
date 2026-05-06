/**
 * Macro / economy panel-family registry. Mirrors panels/news +
 * panels/intel + panels/markets registry shapes.
 */

import { type ComponentType } from "react";

import { EconIndicatorTile } from "./EconIndicatorTile";
import { CpiBreakdown } from "./CpiBreakdown";
import { CountryEconCard } from "./CountryEconCard";

export { EconIndicatorTile, CpiBreakdown, CountryEconCard };
export type { EconIndicatorTileProps, IndicatorTone } from "./EconIndicatorTile";
export type { CpiBreakdownProps, CpiComponent } from "./CpiBreakdown";
export type { CountryEconCardProps, CountryEconSummary } from "./CountryEconCard";

export type Variant = "base" | "tech" | "finance" | "commodity" | "happy";
export type Tier = 0 | 1 | 2 | 3;

export interface PanelDescriptor {
  id: string;
  title: string;
  blurb: string;
  component: ComponentType<unknown>;
  cacheKeys: string[];
  minTier: Tier;
  variants: Variant[] | "*";
}

const REGISTRY: PanelDescriptor[] = [];
const REGISTERED_IDS = new Set<string>();

export function registerPanel(descriptor: PanelDescriptor): void {
  if (REGISTERED_IDS.has(descriptor.id)) {
    throw new Error(
      `panels/macro: duplicate panel id ${JSON.stringify(descriptor.id)}`,
    );
  }
  REGISTERED_IDS.add(descriptor.id);
  REGISTRY.push(descriptor);
}

export function listPanels(): PanelDescriptor[] {
  return [...REGISTRY];
}

export function findPanel(id: string): PanelDescriptor | undefined {
  return REGISTRY.find((p) => p.id === id);
}

export function panelsForVariant(variant: Variant): PanelDescriptor[] {
  return REGISTRY.filter(
    (p) => p.variants === "*" || p.variants.includes(variant),
  );
}

export function panelsAvailableForTier(tier: Tier): PanelDescriptor[] {
  return REGISTRY.filter((p) => p.minTier <= tier);
}

export function _resetRegistryForTests(): void {
  REGISTRY.length = 0;
  REGISTERED_IDS.clear();
}
