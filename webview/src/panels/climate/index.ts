/**
 * Climate / nature panel-family registry. Mirrors panels/macro
 * shape — same registerPanel / listPanels surface.
 */

import { type ComponentType } from "react";

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
      `panels/climate: duplicate panel id ${JSON.stringify(descriptor.id)}`,
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

/** Test-only registry reset. */
export function _resetRegistryForTests(): void {
  REGISTRY.length = 0;
  REGISTERED_IDS.clear();
}
