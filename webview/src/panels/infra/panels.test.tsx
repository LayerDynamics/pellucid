import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";
import { type ReactElement } from "react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";

import { InfraPanel, INFRA_PANEL_ID } from "./InfraPanel";
import { InternetOutagesPanel, INTERNET_OUTAGES_PANEL_ID } from "./InternetOutagesPanel";
import { CyberIncidentsPanel, CYBER_INCIDENTS_PANEL_ID } from "./CyberIncidentsPanel";
import { CveTrendingPanel, CVE_TRENDING_PANEL_ID } from "./CveTrendingPanel";
import { ActiveCampaignsPanel, ACTIVE_CAMPAIGNS_PANEL_ID } from "./ActiveCampaignsPanel";
import { CloudStatusPanel, CLOUD_STATUS_PANEL_ID } from "./CloudStatusPanel";

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
    id: INFRA_PANEL_ID,
    Element: () => (
      <InfraPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              tiles: [{ code: "CLOUD", label: "Cloud incidents", value: "1/3", tone: "neutral" }],
              availableTiles: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/infra/summary").loadInfraSummary>>
        }
      />
    ),
    expectedField: '[data-component="InfraTile"][data-code="CLOUD"]',
  },
  {
    id: INTERNET_OUTAGES_PANEL_ID,
    Element: () => (
      <InternetOutagesPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ provider: "Cloudflare", region: "global", status: "degraded", startedAtMs: NOW, affectedAsCount: 12 }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/infra/internet-outages").loadInternetOutages>>
        }
      />
    ),
    expectedField: '[data-component="OutageRow"][data-provider="Cloudflare"]',
  },
  {
    id: CYBER_INCIDENTS_PANEL_ID,
    Element: () => (
      <CyberIncidentsPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "i1", title: "Ransomware: ACME Co.", severity: "high", source: "BleepingComputer", publishedAtMs: NOW }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/infra/cyber-incidents").loadCyberIncidents>>
        }
      />
    ),
    expectedField: '[data-component="IncidentRow"][data-id="i1"]',
  },
  {
    id: CVE_TRENDING_PANEL_ID,
    Element: () => (
      <CveTrendingPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ cveId: "CVE-2026-1234", summary: "rce in lib", cvssScore: 9.8, severity: "Critical", publishedAtMs: NOW }],
              maxCvss: 9.8,
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/infra/cve-trending").loadCveTrending>>
        }
      />
    ),
    expectedField: '[data-component="CveRow"][data-cve="CVE-2026-1234"]',
  },
  {
    id: ACTIVE_CAMPAIGNS_PANEL_ID,
    Element: () => (
      <ActiveCampaignsPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ id: "c1", title: "APT29 phishing", actor: "APT29", severity: "high", sectors: ["finance"] }],
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/infra/active-campaigns").loadActiveCampaigns>>
        }
      />
    ),
    expectedField: '[data-component="CampaignRow"][data-id="c1"]',
  },
  {
    id: CLOUD_STATUS_PANEL_ID,
    Element: () => (
      <CloudStatusPanel
        load={async () =>
          ({
            kind: "ready",
            response: {
              rows: [{ provider: "AWS", component: "S3", status: "degraded", region: "us-east-1" }],
              incidentCount: 1,
              total: 1,
              assembledAtMs: NOW,
              stale: false,
            },
          }) as Awaited<ReturnType<typeof import("../../data/loaders/infra/cloud-status").loadCloudStatus>>
        }
      />
    ),
    expectedField: '[data-component="CloudStatusRow"][data-provider="AWS"]',
  },
];

describe("infra panels — view-state matrix", () => {
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
