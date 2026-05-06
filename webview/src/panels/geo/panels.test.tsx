import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";
import { type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";

import { UcdpEventsPanel, UCDP_EVENTS_PANEL_ID } from "./UcdpEventsPanel";
import { StrategicPosturePanel, STRATEGIC_POSTURE_PANEL_ID } from "./StrategicPosturePanel";
import { StrategicRiskPanel, STRATEGIC_RISK_PANEL_ID } from "./StrategicRiskPanel";
import { MilitaryCorrelationPanel, MILITARY_CORRELATION_PANEL_ID } from "./MilitaryCorrelationPanel";
import { EscalationCorrelationPanel, ESCALATION_CORRELATION_PANEL_ID } from "./EscalationCorrelationPanel";
import { ThermalEscalationPanel, THERMAL_ESCALATION_PANEL_ID } from "./ThermalEscalationPanel";
import { DefensePatentsPanel, DEFENSE_PATENTS_PANEL_ID } from "./DefensePatentsPanel";
import { SanctionsPressurePanel, SANCTIONS_PRESSURE_PANEL_ID } from "./SanctionsPressurePanel";
import { SupplyChainPanel, SUPPLY_CHAIN_PANEL_ID } from "./SupplyChainPanel";
import { TradePolicyPanel, TRADE_POLICY_PANEL_ID } from "./TradePolicyPanel";

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
    id: UCDP_EVENTS_PANEL_ID,
    Element: () => (
      <UcdpEventsPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "u1", country: "Ukraine", actor1: "GovOfRus", actor2: "GovOfUkr", fatalities: 12, lat: 50.4, lon: 30.5, occurredAt: "2026-04-29T01:00:00Z" }],
              totalFatalities: 12,
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/geo/ucdp-events").loadUcdpEvents>>
        }
      />
    ),
    expectedField: '[data-component="UcdpEventRow"][data-id="u1"]',
  },
  {
    id: STRATEGIC_POSTURE_PANEL_ID,
    Element: () => (
      <StrategicPosturePanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ theater: "EUCOM", force: "USAREUR", readiness: "C-1", headcount: 25000 }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/geo/strategic-posture").loadStrategicPosture>>
        }
      />
    ),
    expectedField: '[data-component="PostureRow"][data-theater="EUCOM"]',
  },
  {
    id: STRATEGIC_RISK_PANEL_ID,
    Element: () => (
      <StrategicRiskPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              level: "elevated",
              score: 45,
              headline: "Strategic risk: elevated",
              rationale: "UCDP 24h: 80 fatalities",
              ucdpFatalities24h: 80,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/geo/strategic-risk").loadStrategicRisk>>
        }
      />
    ),
    expectedField: '[data-component="StrategicRiskGauge"]',
  },
  {
    id: MILITARY_CORRELATION_PANEL_ID,
    Element: () => (
      <MilitaryCorrelationPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ theater: "EUCOM", readiness: "C-1", headcount: 25000, fatalities24h: 12, deployments: 3, correlation: 70 }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/geo/military-correlation").loadMilitaryCorrelation>>
        }
      />
    ),
    expectedField: '[data-component="CorrelationRow"][data-theater="EUCOM"]',
  },
  {
    id: ESCALATION_CORRELATION_PANEL_ID,
    Element: () => (
      <EscalationCorrelationPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ zone: "Ukraine", thermalAnomalies: 2, fatalities24h: 50, escalation: 54 }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/geo/escalation-correlation").loadEscalationCorrelation>>
        }
      />
    ),
    expectedField: '[data-component="EscalationRow"][data-zone="Ukraine"]',
  },
  {
    id: THERMAL_ESCALATION_PANEL_ID,
    Element: () => (
      <ThermalEscalationPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "a", lat: 50.0, lon: 30.0, brightnessK: 410, confidence: 90, zone: "Ukraine", acquiredAt: "2026-04-29T12:00:00Z" }],
              highConfidenceCount: 1,
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/geo/thermal-escalation").loadThermalEscalation>>
        }
      />
    ),
    expectedField: '[data-component="ThermalRow"][data-id="a"]',
  },
  {
    id: DEFENSE_PATENTS_PANEL_ID,
    Element: () => (
      <DefensePatentsPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ cpcClass: "B64G", label: "B64G class", filings30d: 220, yoyPct: 41, topFiler: "Lockheed Martin" }],
              totalFilings30d: 220,
              period: "2026-W18",
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/geo/defense-patents").loadDefensePatents>>
        }
      />
    ),
    expectedField: '[data-component="PatentClassRow"][data-cpc="B64G"]',
  },
  {
    id: SANCTIONS_PRESSURE_PANEL_ID,
    Element: () => (
      <SanctionsPressurePanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ authority: "OFAC", entity: "Test Co.", entityType: "company", jurisdiction: "RU", listedOn: "2026-04-29", programme: "RUSSIA" }],
              byAuthority: [["OFAC", 1]],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/geo/sanctions-pressure").loadSanctionsPressure>>
        }
      />
    ),
    expectedField: '[data-component="AuthorityChip"][data-authority="OFAC"]',
  },
  {
    id: SUPPLY_CHAIN_PANEL_ID,
    Element: () => (
      <SupplyChainPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              tiles: [{ code: "STRESS", label: "Stress index", value: "0.72", tone: "negative" }],
              availableTiles: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/geo/supply-chain").loadSupplyChain>>
        }
      />
    ),
    expectedField: '[data-component="SupplyChainTile"][data-code="STRESS"]',
  },
  {
    id: TRADE_POLICY_PANEL_ID,
    Element: () => (
      <TradePolicyPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ authority: "USTR", origin: "CN", destination: "US", hsCode: "8542", product: "Semiconductors", rateDeltaPp: 25, effective: "2026-04-30", headline: "S301 raise" }],
              totalRateDeltaPp: 25,
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/geo/trade-policy").loadTradePolicy>>
        }
      />
    ),
    expectedField: '[data-component="TariffRow"][data-authority="USTR"]',
  },
];

describe("geo panels — view-state matrix", () => {
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
