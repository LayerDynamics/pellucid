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

describe("panels/macro registry", () => {
  test("registerPanel appends + listPanels reflects", () => {
    expect(listPanels()).toEqual([]);
    const d = makeDescriptor("macro/economic");
    registerPanel(d);
    expect(listPanels()).toEqual([d]);
  });
  test("registerPanel throws on duplicate id", () => {
    registerPanel(makeDescriptor("macro/economic"));
    expect(() => registerPanel(makeDescriptor("macro/economic"))).toThrow(
      /duplicate panel id/,
    );
  });
  test("findPanel returns matching", () => {
    registerPanel(makeDescriptor("macro/economic"));
    expect(findPanel("macro/economic")?.id).toBe("macro/economic");
    expect(findPanel("macro/missing")).toBeUndefined();
  });
  test("panelsForVariant filters", () => {
    registerPanel(makeDescriptor("macro/all", { variants: "*" }));
    registerPanel(
      makeDescriptor("macro/finance-only", {
        variants: ["finance" as Variant],
      }),
    );
    const out = panelsForVariant("finance").map((p) => p.id).sort();
    expect(out).toEqual(["macro/all", "macro/finance-only"]);
  });
  test("panelsAvailableForTier filters", () => {
    registerPanel(makeDescriptor("macro/free", { minTier: 0 as Tier }));
    registerPanel(makeDescriptor("macro/pro", { minTier: 2 as Tier }));
    const out = panelsAvailableForTier(1).map((p) => p.id).sort();
    expect(out).toEqual(["macro/free"]);
  });
});
