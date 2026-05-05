import { afterEach, describe, expect, test } from "bun:test";
import {
  cleanup,
  render,
  screen,
  waitFor,
} from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  DEFAULT_LIMIT,
  MARKET_PANEL_ID,
  MarketPanel,
  REQUIRED_TIER,
  formatAssembledAt,
  summaryTiles,
} from "./MarketPanel";
import type {
  ListMarketQuotesOutcome,
  ListMarketQuotesResponse,
  MarketQuote,
} from "../../data/loaders/market/list-market-quotes";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const NOW_MS = Date.UTC(2026, 4, 5, 12, 0, 0);
const ASSEMBLED_AT_MS = Date.UTC(2026, 4, 5, 11, 59, 30);

function quote(symbol: string, price: number, prev: number): MarketQuote {
  const pct = prev === 0 ? 0 : ((price - prev) / prev) * 100;
  return {
    symbol,
    price,
    previousClose: prev,
    percentChange: pct,
    currency: "USD",
    exchange: "PCX",
    regularMarketTimeMs: 1_714_060_800_000,
  };
}

const SAMPLE: ListMarketQuotesResponse = {
  rows: [
    quote("SPY", 524, 522), // +0.38%
    quote("QQQ", 460, 458), // +0.44%
    quote("DIA", 388, 392), // -1.02%
  ],
  assembledAtMs: ASSEMBLED_AT_MS,
  total: 3,
  stale: false,
};

function readyLoader(response: ListMarketQuotesResponse) {
  return async () =>
    ({ kind: "ready", response }) as ListMarketQuotesOutcome;
}

describe("MarketPanel — view states", () => {
  test("loading → ready transitions and renders one watchlist row per quote", async () => {
    render(<MarketPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    expect(screen.getByText("Loading market quotes…")).toBeDefined();
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const rows = document.querySelectorAll(
      '[data-component="WatchlistRow"]',
    );
    expect(rows.length).toBe(3);
  });

  test("renders the 4-tile metric grid with basket / median / gainer / loser", async () => {
    render(<MarketPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const tiles = document.querySelectorAll('[data-component="MetricTile"]');
    expect(tiles.length).toBe(4);
    const ids = Array.from(tiles).map((t) => t.getAttribute("data-tile-id"));
    expect(ids).toEqual(["basket", "median", "gainer", "loser"]);
  });

  test("empty rows → empty-state row, not the ready block", async () => {
    render(
      <MarketPanel
        load={readyLoader({
          rows: [],
          assembledAtMs: ASSEMBLED_AT_MS,
          total: 0,
          stale: false,
        })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="empty"]')).not.toBeNull();
    });
  });

  test("503 outage path renders the outage banner with retry-after", async () => {
    render(
      <MarketPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as ListMarketQuotesOutcome
        }
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
    expect(screen.getByText(/Retry available in 30s/)).toBeDefined();
  });

  test("generic error renders message + error-code data attribute", async () => {
    render(
      <MarketPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as ListMarketQuotesOutcome
        }
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot footer surfaces 'Showing cached snapshot.' hint", async () => {
    render(
      <MarketPanel
        load={readyLoader({ ...SAMPLE, stale: true })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });

  test("assembled-at footer renders a relative timestamp", async () => {
    render(<MarketPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    await waitFor(() => {
      const el = document.querySelector('[data-field="assembled-at"]');
      // 30 seconds before NOW_MS → "30s ago".
      expect(el?.textContent).toContain("30s ago");
    });
  });
});

describe("MarketPanel — registration + lifecycle", () => {
  test("registers itself with usePanelStore on mount", () => {
    render(<MarketPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    const layout = usePanelStore.getState().getLayout(MARKET_PANEL_ID);
    expect(layout?.id).toBe(MARKET_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(MARKET_PANEL_ID);
    const { container } = render(
      <MarketPanel load={readyLoader(SAMPLE)} now={NOW_MS} />,
    );
    expect(container.querySelector("[data-panel-id]")).toBeNull();
  });

  test("DEFAULT_LIMIT is 50, REQUIRED_TIER is 0", () => {
    expect(DEFAULT_LIMIT).toBe(50);
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("summaryTiles", () => {
  test("returns [] for empty input", () => {
    expect(summaryTiles([])).toEqual([]);
  });

  test("computes basket / median / top-gainer / top-loser", () => {
    const tiles = summaryTiles(SAMPLE.rows);
    expect(tiles[0]?.value).toBe("3");
    // Median % change of [-1.02, +0.38, +0.44] is +0.38 (middle element).
    expect(tiles[1]?.value).toContain("0.38");
    // Top gainer is QQQ (+0.44%).
    expect(tiles[2]?.value).toBe("QQQ");
    // Top loser is DIA (-1.02%).
    expect(tiles[3]?.value).toBe("DIA");
  });

  test("collapses to 3 tiles when basket has 1 row (gainer == loser)", () => {
    const tiles = summaryTiles([quote("SPY", 100, 99)]);
    // basket + median + gainer (loser deduped).
    expect(tiles.length).toBe(3);
    expect(tiles.map((t) => t.id)).toEqual(["basket", "median", "gainer"]);
  });
});

describe("formatAssembledAt", () => {
  test("non-finite or zero → em-dash", () => {
    expect(formatAssembledAt(0, NOW_MS)).toBe("—");
    expect(formatAssembledAt(Number.NaN, NOW_MS)).toBe("—");
  });

  test("seconds / minutes / hours / fallback formatting", () => {
    const now = Date.UTC(2026, 4, 5, 12, 0, 0);
    expect(formatAssembledAt(now - 30_000, now)).toBe("30s ago");
    expect(formatAssembledAt(now - 5 * 60_000, now)).toBe("5m ago");
    expect(formatAssembledAt(now - 3 * 3_600_000, now)).toBe("3h ago");
    // > 24h: HH:MM UTC fallback.
    const old = Date.UTC(2026, 4, 1, 5, 30, 0);
    expect(formatAssembledAt(old, now)).toBe("05:30 UTC");
  });
});
