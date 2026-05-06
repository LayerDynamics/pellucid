import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";
import { type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";

import { ClimatePanel, CLIMATE_PANEL_ID } from "./ClimatePanel";
import { WildfirePanel, WILDFIRE_PANEL_ID } from "./WildfirePanel";
import { EarthquakesPanel, EARTHQUAKES_PANEL_ID } from "./EarthquakesPanel";
import { AirQualityPanel, AIR_QUALITY_PANEL_ID } from "./AirQualityPanel";
import { VolcanoActivityPanel, VOLCANO_PANEL_ID } from "./VolcanoActivityPanel";
import { WeatherAlertsPanel, WEATHER_ALERTS_PANEL_ID } from "./WeatherAlertsPanel";
import { ClimateAnomaliesPanel, CLIMATE_ANOMALIES_PANEL_ID } from "./ClimateAnomaliesPanel";
import { NaturalEventsPanel, NATURAL_EVENTS_PANEL_ID } from "./NaturalEventsPanel";

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
    id: CLIMATE_PANEL_ID,
    Element: () => (
      <ClimatePanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              tiles: [{ code: "ANOMALY", label: "Global anomaly", value: "+1.42 °C", tone: "negative" }],
              availableTiles: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/climate/summary").loadClimateSummary>>
        }
      />
    ),
    expectedField: '[data-component="ClimateSummaryTile"][data-code="ANOMALY"]',
  },
  {
    id: WILDFIRE_PANEL_ID,
    Element: () => (
      <WildfirePanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ label: "Park Fire", region: "CA", lat: 39.6, lon: -121.5, acresBurned: 426400, containmentPct: 100 }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/climate/wildfire").loadWildfire>>
        }
      />
    ),
    expectedField: '[data-component="WildfireRow"]',
  },
  {
    id: EARTHQUAKES_PANEL_ID,
    Element: () => (
      <EarthquakesPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "us2", place: "Japan", mag: 6.1, depth: 50, lat: 35, lon: 140, occurredAtMs: NOW }],
              maxMag: 6.1,
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/climate/earthquakes").loadEarthquakes>>
        }
      />
    ),
    expectedField: '[data-component="QuakeRow"][data-id="us2"]',
  },
  {
    id: AIR_QUALITY_PANEL_ID,
    Element: () => (
      <AirQualityPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ city: "Delhi", country: "IN", aqi: 250, pollutant: "pm25", value: 145, unit: "µg/m³" }],
              worstAqi: 250,
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/climate/air-quality").loadAirQuality>>
        }
      />
    ),
    expectedField: '[data-component="AirQualityRow"][data-city="Delhi"]',
  },
  {
    id: VOLCANO_PANEL_ID,
    Element: () => (
      <VolcanoActivityPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "v1", name: "Kilauea", country: "US", status: "erupting", lat: 19.4, lon: -155.3 }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/climate/volcano").loadVolcanoActivity>>
        }
      />
    ),
    expectedField: '[data-component="VolcanoRow"][data-id="v1"]',
  },
  {
    id: WEATHER_ALERTS_PANEL_ID,
    Element: () => (
      <WeatherAlertsPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ event: "Tornado Warning", area: "OK", severity: "Extreme", urgency: "Immediate", effective: "2026-04-29T00:00:00Z", expires: "2026-04-29T01:00:00Z" }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/climate/noaa-alerts").loadNoaaAlerts>>
        }
      />
    ),
    expectedField: '[data-component="AlertRow"][data-severity="Extreme"]',
  },
  {
    id: CLIMATE_ANOMALIES_PANEL_ID,
    Element: () => (
      <ClimateAnomaliesPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              globalAnomalyC: 1.42,
              period: "2026-04",
              records: [{ stationId: "USC001", label: "Death Valley", recordClass: "high-temp", value: 134.0, setOn: "2026-04-15" }],
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/climate/anomalies").loadClimateAnomalies>>
        }
      />
    ),
    expectedField: '[data-component="StationRecordRow"][data-station="USC001"]',
  },
  {
    id: NATURAL_EVENTS_PANEL_ID,
    Element: () => (
      <NaturalEventsPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              tiles: [{ kind: "earthquake", label: "M6.1 Japan", region: "Japan", severity: "severe" }],
              totalEvents: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/climate/natural-events").loadNaturalEvents>>
        }
      />
    ),
    expectedField: '[data-component="EventTile"][data-kind="earthquake"]',
  },
];

describe("climate panels — view-state matrix", () => {
  for (const c of CASES) {
    test(`${c.id} → ready`, async () => {
      render(<c.Element />);
      await waitFor(() => {
        expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
      });
      if (c.expectedField) expect(document.querySelector(c.expectedField)).not.toBeNull();
    });

    test(`${c.id} → 503 outage`, async () => {
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
