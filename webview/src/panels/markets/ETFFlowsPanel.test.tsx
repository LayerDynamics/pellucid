import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  ETFFlowsPanel,
  ETF_FLOWS_PANEL_ID,
  REQUIRED_TIER,
  DEFAULT_SORT,
} from "./ETFFlowsPanel";
import type {
  EtfFlowsOutcome,
  EtfFlowsResponse,
} from "../../data/loaders/market/etf-flows";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const SAMPLE: EtfFlowsResponse = {
  rows: [
    {
      symbol: "SPY",
      latestDollarVolume: 1_500_000_000,
      avgDollarVolume: 1_000_000_000,
      activityRatio: 1.5,
      latestSessionTs: 1_700_000_000,
    },
    {
      symbol: "QQQ",
      latestDollarVolume: 800_000_000,
      avgDollarVolume: 1_000_000_000,
      activityRatio: 0.8,
      latestSessionTs: 1_700_000_000,
    },
  ],
  lookbackDays: 5,
  total: 2,
  assembledAtMs: 1_700_000_000_000,
  stale: false,
};

function readyLoader(response: EtfFlowsResponse) {
  return async () => ({ kind: "ready", response }) as EtfFlowsOutcome;
}

describe("ETFFlowsPanel — view states", () => {
  test("loading → ready renders one row per response row", async () => {
    render(<ETFFlowsPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const rows = document.querySelectorAll('tr[data-row-symbol]');
    expect(rows.length).toBe(2);
    expect(document.querySelector('[data-row-symbol="SPY"]')).not.toBeNull();
  });

  test("empty rows render the empty-state row", async () => {
    render(
      <ETFFlowsPanel load={readyLoader({ ...SAMPLE, rows: [], total: 0 })} />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="empty"]')).not.toBeNull();
    });
    expect(document.querySelector('[data-state="ready"]')).toBeNull();
  });

  test("503 outage path renders the outage banner with retry-after", async () => {
    render(
      <ETFFlowsPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as EtfFlowsOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("generic error renders error banner with code data attribute", async () => {
    render(
      <ETFFlowsPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as EtfFlowsOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale=true shows the cached-snapshot footnote", async () => {
    render(
      <ETFFlowsPanel load={readyLoader({ ...SAMPLE, stale: true })} />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });

  test("activity ratio cell carries the data-field selector for assertions", async () => {
    render(<ETFFlowsPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      const cells = document.querySelectorAll('[data-field="activity-ratio"]');
      expect(cells.length).toBe(2);
    });
  });
});

describe("ETFFlowsPanel — registration + sort control", () => {
  test("registers itself with usePanelStore on mount", () => {
    render(<ETFFlowsPanel load={readyLoader(SAMPLE)} />);
    const layout = usePanelStore.getState().getLayout(ETF_FLOWS_PANEL_ID);
    expect(layout).toBeDefined();
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(ETF_FLOWS_PANEL_ID);
    const { container } = render(<ETFFlowsPanel load={readyLoader(SAMPLE)} />);
    expect(container.querySelector('[data-panel-id]')).toBeNull();
  });

  test("REQUIRED_TIER is 0 (anonymous)", () => {
    expect(REQUIRED_TIER).toBe(0);
    expect(DEFAULT_SORT).toBe("activity-ratio-desc");
  });

  test("changing the sort dropdown refetches with new ?sort", async () => {
    const seen: Array<{ sort?: string }> = [];
    const loader = async (q?: { sort?: string }) => {
      seen.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as EtfFlowsOutcome;
    };
    render(<ETFFlowsPanel load={loader} />);
    await waitFor(() => seen.length === 1);
    const select = document.querySelector(
      'select[data-field="sort-select"]',
    ) as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "dollar-volume-desc" } });
    await waitFor(() => seen.length >= 2);
    const last = seen[seen.length - 1];
    expect(last?.sort).toBe("dollar-volume-desc");
  });
});
