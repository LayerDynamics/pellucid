import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";
import { type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";

import { EnergyComplexPanel, ENERGY_COMPLEX_PANEL_ID } from "./EnergyComplexPanel";
import { EnergyCrisisPanel, ENERGY_CRISIS_PANEL_ID } from "./EnergyCrisisPanel";
import { OilInventoriesPanel, OIL_INVENTORIES_PANEL_ID } from "./OilInventoriesPanel";
import { HormuzPanel, HORMUZ_PANEL_ID } from "./HormuzPanel";
import { RenewableEnergyPanel, RENEWABLE_ENERGY_PANEL_ID } from "./RenewableEnergyPanel";
import { GoldIntelligencePanel, GOLD_INTELLIGENCE_PANEL_ID } from "./GoldIntelligencePanel";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const NOW = Date.UTC(2026, 4, 5, 12, 30, 0);

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
  expectedField?: string;
}

const CASES: PanelCase[] = [
  {
    id: ENERGY_COMPLEX_PANEL_ID,
    Element: () => (
      <EnergyComplexPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              tiles: [{ code: "FUEL", label: "Avg fuel price", value: "$3.45/gal", subline: "5 regions", tone: "neutral" }],
              availableTiles: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/energy/complex").loadEnergyComplex>>
        }
      />
    ),
    expectedField: '[data-component="EnergyComplexTile"][data-code="FUEL"]',
  },
  {
    id: ENERGY_CRISIS_PANEL_ID,
    Element: () => (
      <EnergyCrisisPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              level: "elevated",
              score: 45,
              headline: "Energy crisis risk: elevated",
              rationale: "EU gas storage at 60% full",
              gasStoragePct: 60,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/energy/crisis").loadEnergyCrisis>>
        }
      />
    ),
    expectedField: '[data-component="CrisisGauge"]',
  },
  {
    id: OIL_INVENTORIES_PANEL_ID,
    Element: () => (
      <OilInventoriesPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ product: "crude", period: "2026-04-26", valueMb: 437.5, wowDeltaMb: -1.2 }],
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/energy/oil-inventories").loadOilInventories>>
        }
      />
    ),
    expectedField: '[data-component="OilStocksRow"][data-product="crude"]',
  },
  {
    id: HORMUZ_PANEL_ID,
    Element: () => (
      <HormuzPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ product: "crude", mbPerDay: 14, sharePct: 70 }],
              totalMbPerDay: 20,
              period: "2026-04",
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/energy/hormuz").loadHormuz>>
        }
      />
    ),
    expectedField: '[data-component="HormuzRow"][data-product="crude"]',
  },
  {
    id: RENEWABLE_ENERGY_PANEL_ID,
    Element: () => (
      <RenewableEnergyPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ source: "solar", capacityGw: 1500, yoyPct: 22, sharePct: 39 }],
              totalCapacityGw: 3800,
              period: "2026-Q1",
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/energy/renewable").loadRenewableMix>>
        }
      />
    ),
    expectedField: '[data-component="RenewableRow"][data-source="solar"]',
  },
  {
    id: GOLD_INTELLIGENCE_PANEL_ID,
    Element: () => (
      <GoldIntelligencePanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ ticker: "GLD", region: "NA", netFlowMillionUsd: 120.5, tonneHoldings: 850 }],
              totalNetFlowMillionUsd: 120.5,
              totalTonneHoldings: 850,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/energy/gold-intelligence").loadGoldIntelligence>>
        }
      />
    ),
    expectedField: '[data-component="GoldEtfRow"][data-ticker="GLD"]',
  },
];

describe("energy panels — view-state matrix", () => {
  for (const c of CASES) {
    test(`${c.id} → ready renders state`, async () => {
      render(<c.Element />);
      await waitFor(() => {
        expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
      });
      if (c.expectedField) expect(document.querySelector(c.expectedField)).not.toBeNull();
    });

    test(`${c.id} → 503 outage renders outage banner`, async () => {
      const Panel = c.Element().type as (props: { load: typeof outage }) => ReactElement;
      render(<Panel load={outage as never} />);
      await waitFor(() => {
        expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
      });
    });

    test(`${c.id} → cache_failure renders error banner`, async () => {
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
