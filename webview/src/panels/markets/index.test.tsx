import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { type ComponentType } from "react";

import {
  _resetRegistryForTests,
  findPanel,
  listPanels,
  panelsAvailableForTier,
  panelsForVariant,
  registerPanel,
  type PanelDescriptor,
  type Tier,
  type Variant,
} from "./index";

const NoopComponent: ComponentType<unknown> = () => null;

function makeDescriptor(
  id: string,
  overrides: Partial<PanelDescriptor> = {},
): PanelDescriptor {
  return {
    id,
    title: id,
    blurb: `${id} blurb`,
    component: NoopComponent,
    cacheKeys: [`${id}:v1`],
    minTier: 0,
    variants: "*",
    ...overrides,
  };
}

beforeEach(() => _resetRegistryForTests());
afterEach(() => _resetRegistryForTests());

describe("panels/markets registry", () => {
  test("registerPanel appends descriptor to listPanels()", () => {
    expect(listPanels()).toEqual([]);
    const d = makeDescriptor("markets/quotes");
    registerPanel(d);
    expect(listPanels()).toEqual([d]);
  });

  test("registerPanel throws on duplicate id", () => {
    registerPanel(makeDescriptor("markets/quotes"));
    expect(() => registerPanel(makeDescriptor("markets/quotes"))).toThrow(
      /duplicate panel id/,
    );
  });

  test("findPanel returns matching descriptor or undefined", () => {
    registerPanel(makeDescriptor("markets/quotes"));
    expect(findPanel("markets/quotes")?.id).toBe("markets/quotes");
    expect(findPanel("markets/missing")).toBeUndefined();
  });

  test("panelsForVariant filters by variant scope", () => {
    registerPanel(makeDescriptor("markets/all", { variants: "*" }));
    registerPanel(
      makeDescriptor("markets/finance-only", {
        variants: ["finance" as Variant],
      }),
    );
    registerPanel(
      makeDescriptor("markets/tech-only", { variants: ["tech" as Variant] }),
    );
    const finance = panelsForVariant("finance");
    const ids = finance.map((p) => p.id).sort();
    expect(ids).toEqual(["markets/all", "markets/finance-only"]);
  });

  test("panelsAvailableForTier filters by minTier", () => {
    registerPanel(makeDescriptor("markets/free", { minTier: 0 as Tier }));
    registerPanel(makeDescriptor("markets/pro", { minTier: 1 as Tier }));
    registerPanel(makeDescriptor("markets/api", { minTier: 2 as Tier }));
    const tier1 = panelsAvailableForTier(1).map((p) => p.id).sort();
    expect(tier1).toEqual(["markets/free", "markets/pro"]);
  });

  test("listPanels returns a fresh array snapshot", () => {
    registerPanel(makeDescriptor("markets/quotes"));
    const a = listPanels();
    const b = listPanels();
    expect(a).not.toBe(b);
    expect(a).toEqual(b);
  });
});
