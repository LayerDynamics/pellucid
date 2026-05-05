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

describe("registerPanel", () => {
  test("appends descriptor to listPanels()", () => {
    expect(listPanels()).toEqual([]);
    const d = makeDescriptor("news/feed");
    registerPanel(d);
    expect(listPanels()).toEqual([d]);
  });

  test("preserves registration order", () => {
    registerPanel(makeDescriptor("news/feed"));
    registerPanel(makeDescriptor("news/live"));
    registerPanel(makeDescriptor("intel/gdelt"));
    expect(listPanels().map((p) => p.id)).toEqual([
      "news/feed",
      "news/live",
      "intel/gdelt",
    ]);
  });

  test("duplicate id throws", () => {
    registerPanel(makeDescriptor("news/feed"));
    expect(() => registerPanel(makeDescriptor("news/feed"))).toThrow(
      /duplicate panel id/,
    );
  });

  test("listPanels returns a defensive copy", () => {
    registerPanel(makeDescriptor("news/feed"));
    const snapshot = listPanels();
    snapshot.length = 0; // mutate the returned array
    expect(listPanels().length).toBe(1); // registry unchanged
  });
});

describe("findPanel", () => {
  test("returns descriptor when registered", () => {
    const d = makeDescriptor("intel/regional");
    registerPanel(d);
    expect(findPanel("intel/regional")).toEqual(d);
  });

  test("returns undefined when not registered", () => {
    registerPanel(makeDescriptor("news/feed"));
    expect(findPanel("does-not-exist")).toBeUndefined();
  });
});

describe("panelsForVariant", () => {
  test("includes wildcard panels for every variant", () => {
    registerPanel(makeDescriptor("news/feed", { variants: "*" }));
    const variants: Variant[] = [
      "base",
      "tech",
      "finance",
      "commodity",
      "happy",
    ];
    for (const v of variants) {
      expect(panelsForVariant(v).map((p) => p.id)).toEqual(["news/feed"]);
    }
  });

  test("filters by explicit variant list", () => {
    registerPanel(
      makeDescriptor("intel/telegram", {
        variants: ["base", "tech", "finance"],
      }),
    );
    expect(panelsForVariant("commodity")).toEqual([]);
    expect(panelsForVariant("happy")).toEqual([]);
    expect(panelsForVariant("base").map((p) => p.id)).toEqual([
      "intel/telegram",
    ]);
  });

  test("mixes wildcard + restricted descriptors correctly", () => {
    registerPanel(makeDescriptor("news/feed", { variants: "*" }));
    registerPanel(
      makeDescriptor("intel/telegram", {
        variants: ["base", "tech", "finance"],
      }),
    );
    expect(panelsForVariant("happy").map((p) => p.id)).toEqual(["news/feed"]);
    expect(panelsForVariant("tech").map((p) => p.id)).toEqual([
      "news/feed",
      "intel/telegram",
    ]);
  });
});

describe("panelsAvailableForTier", () => {
  test("includes lower-tier panels for higher-tier user", () => {
    registerPanel(makeDescriptor("news/feed", { minTier: 0 }));
    registerPanel(makeDescriptor("intel/regional", { minTier: 1 }));
    registerPanel(makeDescriptor("intel/telegram", { minTier: 2 }));
    registerPanel(
      makeDescriptor("admin/internal", { minTier: 3 }),
    );

    const tier3: Tier = 3;
    expect(panelsAvailableForTier(tier3).map((p) => p.id)).toEqual([
      "news/feed",
      "intel/regional",
      "intel/telegram",
      "admin/internal",
    ]);
  });

  test("excludes panels above the user's tier", () => {
    registerPanel(makeDescriptor("news/feed", { minTier: 0 }));
    registerPanel(makeDescriptor("intel/telegram", { minTier: 2 }));
    expect(panelsAvailableForTier(0).map((p) => p.id)).toEqual([
      "news/feed",
    ]);
    expect(panelsAvailableForTier(1).map((p) => p.id)).toEqual([
      "news/feed",
    ]);
    expect(panelsAvailableForTier(2).map((p) => p.id)).toEqual([
      "news/feed",
      "intel/telegram",
    ]);
  });
});

describe("registry exports", () => {
  test("re-exports the four shared sub-components", async () => {
    const mod = await import("./index");
    expect(typeof mod.NewsCard).toBe("function");
    expect(typeof mod.IntelEntityChip).toBe("function");
    expect(typeof mod.BreakingTicker).toBe("function");
    expect(typeof mod.SignalSeverityBadge).toBe("function");
    expect(typeof mod.compareSeverity).toBe("function");
  });
});
