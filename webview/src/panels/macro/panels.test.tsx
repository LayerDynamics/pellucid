import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";
import { type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";

import { EconomicPanel, ECONOMIC_PANEL_ID } from "./EconomicPanel";
import { ConsumerPricesPanel, CONSUMER_PRICES_PANEL_ID } from "./ConsumerPricesPanel";
import { FSIPanel, FSI_PANEL_ID } from "./FSIPanel";
import { MacroSignalsPanel, MACRO_SIGNALS_PANEL_ID, arrow, directionClass } from "./MacroSignalsPanel";
import { MacroTilesPanel, MACRO_TILES_PANEL_ID } from "./MacroTilesPanel";
import { NationalDebtPanel, NATIONAL_DEBT_PANEL_ID } from "./NationalDebtPanel";
import { BigMacPanel, BIG_MAC_PANEL_ID } from "./BigMacPanel";
import { GroceryBasketPanel, GROCERY_BASKET_PANEL_ID } from "./GroceryBasketPanel";
import { FuelPricesPanel, FUEL_PRICES_PANEL_ID } from "./FuelPricesPanel";
import { FaoFoodPriceIndexPanel, FAO_FOOD_PRICE_INDEX_PANEL_ID } from "./FaoFoodPriceIndexPanel";
import { GulfEconomiesPanel, GULF_ECONOMIES_PANEL_ID } from "./GulfEconomiesPanel";

import { formatAssembledAtUtc, labelForCode } from "./labelForCode";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const NOW = Date.UTC(2026, 4, 5, 11, 59, 0);

const outage = async () => ({
  kind: "error" as const,
  code: "bootstrap_upstream_empty",
  message: "x",
  httpStatus: 503,
  retryAfterSecs: 30,
});

const cacheError = async () => ({
  kind: "error" as const,
  code: "cache_failure",
  message: "db",
  httpStatus: 502,
  retryAfterSecs: null,
});

interface PanelCase {
  id: string;
  Element: () => ReactElement;
  readyOutcome: () => Promise<unknown>;
  expectedRow?: string;
  expectedRowCount?: number;
  expectedField?: string;
}

const CASES: PanelCase[] = [
  {
    id: ECONOMIC_PANEL_ID,
    Element: () => (
      <EconomicPanel
        load={async () =>
          ({
            kind: "ready",
            response: { indicators: [{ code: "UNRATE", value: 3.8, period: "2026-04" }], assembledAtMs: NOW, stale: false },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/snapshot").loadEconomicSnapshot>>
        }
      />
    ),
    readyOutcome: async () => ({ kind: "ready", response: { indicators: [], assembledAtMs: NOW, stale: false } }),
  },
  {
    id: CONSUMER_PRICES_PANEL_ID,
    Element: () => (
      <ConsumerPricesPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ region: "US", period: "2026-04", headlineValue: 312.5, yoyPct: 3.2, momPct: 0.3, components: [{ label: "Food", yoyPct: 2.1 }] }],
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/consumer-prices-list").loadCpiList>>
        }
      />
    ),
    readyOutcome: async () => ({ kind: "ready", response: { rows: [], assembledAtMs: NOW, stale: false } }),
    expectedRow: "[data-component=\"CpiRegionRow\"][data-region=\"US\"]",
  },
  {
    id: FSI_PANEL_ID,
    Element: () => (
      <FSIPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              seriesCode: "STLFSI4",
              latest: { date: "2026-04-29", value: 0.5 },
              prior: { date: "2026-04-22", value: 0.4 },
              history: [
                { date: "2026-04-22", value: 0.4 },
                { date: "2026-04-29", value: 0.5 },
              ],
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/financial-stress").loadFinancialStress>>
        }
      />
    ),
    readyOutcome: async () => ({
      kind: "ready",
      response: { seriesCode: "STLFSI4", latest: { date: "2026-04-29", value: 0.5 }, history: [], assembledAtMs: NOW, stale: false },
    }),
    expectedField: '[data-component="FsiSparkline"]',
  },
  {
    id: MACRO_SIGNALS_PANEL_ID,
    Element: () => (
      <MacroSignalsPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              signals: [
                { code: "UNRATE", direction: "rising", headline: "Unemployment rising", rationale: "Latest 3.8 vs prior 3.6 (+0.20)" },
              ],
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/macro-signals").loadMacroSignals>>
        }
      />
    ),
    readyOutcome: async () => ({ kind: "ready", response: { signals: [], assembledAtMs: NOW, stale: false } }),
    expectedRow: '[data-component="MacroSignalRow"][data-code="UNRATE"]',
  },
  {
    id: MACRO_TILES_PANEL_ID,
    Element: () => (
      <MacroTilesPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              tiles: [{ code: "UNRATE", label: "Unemployment", value: "3.8%", tone: "negative" }],
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/macro-tiles").loadMacroTiles>>
        }
      />
    ),
    readyOutcome: async () => ({ kind: "ready", response: { tiles: [], assembledAtMs: NOW, stale: false } }),
    expectedField: '[data-component="EconIndicatorTile"][data-tile-id="UNRATE"]',
  },
  {
    id: NATIONAL_DEBT_PANEL_ID,
    Element: () => (
      <NationalDebtPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              latest: { period: "2026-Q1", totalBillionUsd: 35400, debtToGdpPct: 122.5 },
              qoqDeltaBillionUsd: 400,
              history: [
                { period: "2025-Q4", totalBillionUsd: 35000, debtToGdpPct: 121 },
                { period: "2026-Q1", totalBillionUsd: 35400, debtToGdpPct: 122.5 },
              ],
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/national-debt").loadNationalDebt>>
        }
      />
    ),
    readyOutcome: async () => ({
      kind: "ready",
      response: {
        latest: { period: "2026-Q1", totalBillionUsd: 35400, debtToGdpPct: 122.5 },
        qoqDeltaBillionUsd: 400,
        history: [],
        assembledAtMs: NOW,
        stale: false,
      },
    }),
    expectedField: '[data-component="EconIndicatorTile"][data-tile-id="qoq"]',
  },
  {
    id: BIG_MAC_PANEL_ID,
    Element: () => (
      <BigMacPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              snapshotDate: "2026-01-01",
              rows: [{ iso: "USA", country: "United States", localPrice: 5, currency: "USD", usdPrice: 5, ppp: 5, fxRate: 1, valuationPct: 0 }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/big-mac").loadBigMac>>
        }
      />
    ),
    readyOutcome: async () => ({
      kind: "ready",
      response: { snapshotDate: "2026-01-01", rows: [], total: 0, assembledAtMs: NOW, stale: false },
    }),
    expectedRow: '[data-component="BigMacRow"][data-iso="USA"]',
  },
  {
    id: GROCERY_BASKET_PANEL_ID,
    Element: () => (
      <GroceryBasketPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ iso: "USA", country: "United States", basketUsd: 120, basketYoyPct: 3.2, period: "2026-04" }],
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/grocery-basket").loadGroceryBasket>>
        }
      />
    ),
    readyOutcome: async () => ({ kind: "ready", response: { rows: [], assembledAtMs: NOW, stale: false } }),
    expectedRow: '[data-component="GroceryRow"][data-iso="USA"]',
  },
  {
    id: FUEL_PRICES_PANEL_ID,
    Element: () => (
      <FuelPricesPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ region: "US National", product: "Regular", usdPerGallon: 3.45, weekOverWeekChangePct: -0.5 }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/fuel-prices").loadFuelPrices>>
        }
      />
    ),
    readyOutcome: async () => ({ kind: "ready", response: { rows: [], total: 0, assembledAtMs: NOW, stale: false } }),
    expectedRow: '[data-component="FuelRow"][data-region="US National"]',
  },
  {
    id: FAO_FOOD_PRICE_INDEX_PANEL_ID,
    Element: () => (
      <FaoFoodPriceIndexPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              latest: { period: "2026-04", composite: 120, subindices: [["meat", 110]] },
              history: [{ period: "2026-04", composite: 120, subindices: [["meat", 110]] }],
              yoyPct: 8,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/fao-food-price-index").loadFaoFoodPriceIndex>>
        }
      />
    ),
    readyOutcome: async () => ({
      kind: "ready",
      response: {
        latest: { period: "2026-04", composite: 120, subindices: [] },
        history: [],
        yoyPct: 0,
        assembledAtMs: NOW,
        stale: false,
      },
    }),
    expectedField: '[data-component="EconIndicatorTile"][data-tile-id="fao-composite"]',
  },
  {
    id: GULF_ECONOMIES_PANEL_ID,
    Element: () => (
      <GulfEconomiesPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ iso: "SAU", country: "Saudi Arabia", gdpUsdBillion: 1100, gdpYoyPct: 2.5, inflationYoyPct: 1.8, unemploymentPct: 5, policyRatePct: 5.5, period: "2026-Q1" }],
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/macro/gulf-economies").loadGulfEconomies>>
        }
      />
    ),
    readyOutcome: async () => ({ kind: "ready", response: { rows: [], assembledAtMs: NOW, stale: false } }),
    expectedField: '[data-component="CountryEconCard"][data-iso="SAU"]',
  },
];

describe("macro panels — view-state matrix", () => {
  for (const c of CASES) {
    test(`${c.id} → ready renders state`, async () => {
      render(<c.Element />);
      await waitFor(() => {
        expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
      });
      if (c.expectedRow) expect(document.querySelector(c.expectedRow)).not.toBeNull();
      if (c.expectedField) expect(document.querySelector(c.expectedField)).not.toBeNull();
    });

    test(`${c.id} → 503 outage renders outage banner`, async () => {
      const Panel = c.Element().type as (props: { load: typeof outage }) => ReactElement;
      render(<Panel load={outage as never} />);
      await waitFor(() => {
        expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
      });
    });

    test(`${c.id} → cache_failure renders error banner with code`, async () => {
      const Panel = c.Element().type as (props: { load: typeof cacheError }) => ReactElement;
      render(<Panel load={cacheError as never} />);
      await waitFor(() => {
        const el = document.querySelector('[data-state="error"]');
        expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
      });
    });

    test(`${c.id} → registers with usePanelStore on mount`, async () => {
      render(<c.Element />);
      await waitFor(() => {
        expect(usePanelStore.getState().getLayout(c.id)).toBeDefined();
      });
    });
  }
});

describe("macro shared helpers", () => {
  test("formatAssembledAtUtc edges", () => {
    expect(formatAssembledAtUtc(Date.UTC(2026, 4, 5, 1, 2, 0))).toBe("01:02 UTC");
    expect(formatAssembledAtUtc(0)).toBe("—");
    expect(formatAssembledAtUtc(Number.NaN)).toBe("—");
  });
  test("labelForCode covers code surface", () => {
    expect(labelForCode("invalid_request")).toBe("Invalid request");
    expect(labelForCode("cache_failure")).toBe("Cache failure");
    expect(labelForCode("entitlement_forbidden")).toBe("Premium tier required");
    expect(labelForCode("network")).toBe("Network error");
    expect(labelForCode("nope")).toBe("Error");
  });
  test("MacroSignalsPanel arrow + directionClass", () => {
    expect(arrow("rising")).toBe("▲");
    expect(arrow("falling")).toBe("▼");
    expect(arrow("flat")).toBe("◆");
    expect(directionClass("rising")).toContain("warn");
    expect(directionClass("falling")).toContain("success");
    expect(directionClass("flat")).toContain("muted");
  });
});
