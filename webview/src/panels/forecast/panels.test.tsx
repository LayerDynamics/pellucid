import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";
import { type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";

import { ForecastPanel, FORECAST_PANEL_ID } from "./ForecastPanel";
import { NowCastPanel, NOW_CAST_PANEL_ID } from "./NowCastPanel";
import { PredictionMarketsPanel, PREDICTION_MARKETS_PANEL_ID } from "./PredictionMarketsPanel";
import { ScenarioStatePanel, SCENARIO_STATE_PANEL_ID } from "./ScenarioStatePanel";
import { ScenarioLibraryPanel, SCENARIO_LIBRARY_PANEL_ID } from "./ScenarioLibraryPanel";
import { ExtendedForecastPanel, EXTENDED_FORECAST_PANEL_ID } from "./ExtendedForecastPanel";

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
    id: FORECAST_PANEL_ID,
    Element: () => (
      <ForecastPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              tiles: [{ code: "NOW", label: "Now-cast avg", value: "65%", tone: "neutral" }],
              availableTiles: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/forecast/summary").loadForecastSummary>>
        }
      />
    ),
    expectedField: '[data-component="ForecastTile"][data-code="NOW"]',
  },
  {
    id: NOW_CAST_PANEL_ID,
    Element: () => (
      <NowCastPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "q1", question: "Will X happen?", probability: 0.65, trend: "rising" }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/forecast/now-cast").loadNowCast>>
        }
      />
    ),
    expectedField: '[data-component="NowCastRow"][data-id="q1"]',
  },
  {
    id: PREDICTION_MARKETS_PANEL_ID,
    Element: () => (
      <PredictionMarketsPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "p1", question: "Q?", yes_price: 0.7, volume_usd: 12000, category: "geo" }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/forecast/prediction-markets").loadPredictionMarkets>>
        }
      />
    ),
    expectedField: '[data-component="MarketRow"][data-id="p1"]',
  },
  {
    id: SCENARIO_STATE_PANEL_ID,
    Element: () => (
      <ScenarioStatePanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "a", question: "qa", yesPrice: 0.6, volumeUsd: 10000, category: "geo" }],
              topRow: { id: "a", question: "qa", yesPrice: 0.6, volumeUsd: 10000, category: "geo" },
              weightedAvgYes: 0.6,
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/forecast/scenario-state").loadScenarioState>>
        }
      />
    ),
    expectedField: '[data-component="WeightedAvg"]',
  },
  {
    id: SCENARIO_LIBRARY_PANEL_ID,
    Element: () => (
      <ScenarioLibraryPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "s1", title: "S1", domain: "geo", probability: 0.42 }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/forecast/scenario-library").loadScenarioLibrary>>
        }
      />
    ),
    expectedField: '[data-component="ScenarioRow"][data-id="s1"]',
  },
  {
    id: EXTENDED_FORECAST_PANEL_ID,
    Element: () => (
      <ExtendedForecastPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "e1", question: "Q?", probability: 0.5, horizon: "1m", delta7d: 0.08 }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/forecast/extended").loadExtendedForecast>>
        }
      />
    ),
    expectedField: '[data-component="ExtendedRow"][data-id="e1"]',
  },
];

describe("forecast panels — view-state matrix", () => {
  for (const c of CASES) {
    test(`${c.id} → ready`, async () => {
      render(<c.Element />);
      await waitFor(() => {
        expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
      });
      if (c.expectedField) expect(document.querySelector(c.expectedField)).not.toBeNull();
    });
    test(`${c.id} → 503`, async () => {
      const Panel = c.Element().type as (props: { load: typeof outage }) => ReactElement;
      render(<Panel load={outage as never} />);
      await waitFor(() => {
        expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
      });
    });
    test(`${c.id} → cache_failure`, async () => {
      const Panel = c.Element().type as (props: { load: typeof cacheError }) => ReactElement;
      render(<Panel load={cacheError as never} />);
      await waitFor(() => {
        const el = document.querySelector('[data-state="error"]');
        expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
      });
    });
    test(`${c.id} → registers layout`, async () => {
      render(<c.Element />);
      await waitFor(() => {
        expect(usePanelStore.getState().getLayout(c.id)).toBeDefined();
      });
    });
  }
});
