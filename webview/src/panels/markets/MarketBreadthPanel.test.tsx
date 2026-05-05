import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  MarketBreadthPanel,
  MARKET_BREADTH_PANEL_ID,
  REQUIRED_TIER,
} from "./MarketBreadthPanel";
import type {
  BreadthOutcome,
  BreadthResponse,
} from "../../data/loaders/market/breadth";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const SAMPLE: BreadthResponse = {
  advancers: 5,
  decliners: 2,
  unchanged: 1,
  advanceDeclineLine: 3,
  newHighs: 1,
  newLows: 0,
  topAdvancers: [
    { symbol: "WIN", percentChange: 5.0 },
    { symbol: "OK", percentChange: 1.0 },
  ],
  topDecliners: [{ symbol: "LOSE", percentChange: -2.0 }],
  universe: 8,
  stale: false,
  assembledAtMs: 1_700_000_000_000,
};

function readyLoader(response: BreadthResponse) {
  return async () => ({ kind: "ready", response }) as BreadthOutcome;
}

describe("MarketBreadthPanel — view states", () => {
  test("loading → ready renders metric grid + top tables", async () => {
    render(<MarketBreadthPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const tiles = document.querySelectorAll('[data-component="MetricTile"]');
    // 6 metric tiles: advancers, decliners, A/D line, new highs, new lows, unchanged
    expect(tiles.length).toBe(6);
    expect(
      document.querySelector('[data-table="top-advancers"]'),
    ).not.toBeNull();
    expect(
      document.querySelector('[data-table="top-decliners"]'),
    ).not.toBeNull();
  });

  test("renders one row per top advancer", async () => {
    render(<MarketBreadthPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      const rows = document.querySelectorAll(
        '[data-table="top-advancers"] tbody tr',
      );
      expect(rows.length).toBe(2);
    });
    const win = document.querySelector(
      '[data-table="top-advancers"] [data-row-symbol="WIN"]',
    );
    expect(win).not.toBeNull();
  });

  test("empty top-decliners table renders the empty-state row", async () => {
    render(
      <MarketBreadthPanel
        load={readyLoader({
          ...SAMPLE,
          topDecliners: [],
        })}
      />,
    );
    await waitFor(() => {
      const dec = document.querySelector('[data-table="top-decliners"]');
      expect(dec?.textContent).toContain("No symbols matched.");
    });
  });

  test("503 outage path renders the outage banner with retry-after", async () => {
    render(
      <MarketBreadthPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as BreadthOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("generic error renders error banner with code data attribute", async () => {
    render(
      <MarketBreadthPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as BreadthOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });
});

describe("MarketBreadthPanel — registration + topN control", () => {
  test("registers itself with usePanelStore on mount", () => {
    render(<MarketBreadthPanel load={readyLoader(SAMPLE)} />);
    const layout = usePanelStore.getState().getLayout(MARKET_BREADTH_PANEL_ID);
    expect(layout).toBeDefined();
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(MARKET_BREADTH_PANEL_ID);
    const { container } = render(
      <MarketBreadthPanel load={readyLoader(SAMPLE)} />,
    );
    expect(container.querySelector('[data-panel-id]')).toBeNull();
  });

  test("REQUIRED_TIER is 0 (anonymous)", () => {
    expect(REQUIRED_TIER).toBe(0);
  });

  test("changing topN refetches with the new value", async () => {
    const seen: Array<{ topN?: number }> = [];
    const loader = async (q?: { topN?: number }) => {
      seen.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as BreadthOutcome;
    };
    render(<MarketBreadthPanel load={loader} />);
    await waitFor(() => seen.length === 1);
    const input = document.querySelector(
      'input[data-field="topn-input"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "12" } });
    await waitFor(() => seen.length >= 2);
    const last = seen[seen.length - 1];
    expect(last?.topN).toBe(12);
  });

  test("stale=true shows the cached-snapshot footnote", async () => {
    render(
      <MarketBreadthPanel load={readyLoader({ ...SAMPLE, stale: true })} />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-field="stale"]'),
      ).not.toBeNull();
    });
  });
});
