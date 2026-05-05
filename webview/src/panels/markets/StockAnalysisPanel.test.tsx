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
  DEFAULT_SYMBOL,
  REQUIRED_TIER,
  STOCK_ANALYSIS_PANEL_ID,
  StockAnalysisPanel,
  metricTiles,
} from "./StockAnalysisPanel";
import type {
  AnalyzeStockOutcome,
  AnalyzeStockResponse,
} from "../../data/loaders/market/analyze-stock";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const SAMPLE: AnalyzeStockResponse = {
  symbol: "SPY",
  price: 524,
  previousClose: 522,
  currency: "USD",
  exchange: "PCX",
  regularMarketTimeMs: 1_714_060_800_000,
  metrics: {
    dollarChange: 2,
    percentChange: 0.383,
    trend: "up",
    magnitude: "small",
    rangePosition: 0.5,
  },
  assembledAtMs: 1_700_000_000_000,
  stale: false,
};

function readyLoader(response: AnalyzeStockResponse) {
  return async () =>
    ({ kind: "ready", response }) as AnalyzeStockOutcome;
}

function signedInTier2User() {
  useAuthStore.getState().signIn({
    userId: "u",
    email: "u@example.com",
    clerkSessionToken: "tok",
    entitlements: {
      tier: 2,
      maxDashboards: 5,
      apiAccess: true,
      apiRateLimit: 600,
      prioritySupport: true,
      exportFormats: ["json"],
      validUntilMs: Date.now() + 60_000,
    },
  });
}

describe("StockAnalysisPanel — view states", () => {
  test("anonymous user sees the locked state without invoking the loader", () => {
    let called = false;
    const loader = async (): Promise<AnalyzeStockOutcome> => {
      called = true;
      return { kind: "ready", response: SAMPLE };
    };
    render(<StockAnalysisPanel load={loader} />);
    expect(document.querySelector('[data-state="locked"]')).not.toBeNull();
    expect(called).toBe(false);
  });

  test("REQUIRED_TIER is 2 (premium / API)", () => {
    expect(REQUIRED_TIER).toBe(2);
  });

  test("DEFAULT_SYMBOL is 'SPY'", () => {
    expect(DEFAULT_SYMBOL).toBe("SPY");
  });

  test("tier-2 user sees full analytics card", async () => {
    signedInTier2User();
    render(<StockAnalysisPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    expect(document.querySelector('[data-field="symbol"]')?.textContent).toBe(
      "SPY",
    );
    const tiles = document.querySelectorAll('[data-component="MetricTile"]');
    expect(tiles.length).toBe(4);
  });

  test("range bar fill width matches rangePosition", async () => {
    signedInTier2User();
    render(<StockAnalysisPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(
        document.querySelector('[data-field="range-position-fill"]'),
      ).not.toBeNull();
    });
    const fill = document.querySelector(
      '[data-field="range-position-fill"]',
    ) as HTMLElement;
    expect(fill.style.width).toBe("50.00%");
    const label = document.querySelector(
      '[data-field="range-position-label"]',
    );
    expect(label?.textContent).toBe("50%");
  });

  test("trend footer surfaces the trend + magnitude", async () => {
    signedInTier2User();
    render(<StockAnalysisPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      const t = document.querySelector('[data-field="trend"]');
      expect(t?.textContent).toContain("up");
      expect(t?.textContent).toContain("small");
    });
  });

  test("403 entitlement_forbidden renders the entitlement-blocked state", async () => {
    signedInTier2User();
    render(
      <StockAnalysisPanel
        load={async () =>
          ({
            kind: "error",
            code: "entitlement_forbidden",
            message: "upgrade",
            httpStatus: 403,
            retryAfterSecs: null,
          }) as AnalyzeStockOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="entitlement"]');
      expect(el?.getAttribute("data-error-code")).toBe("entitlement_forbidden");
    });
  });

  test("503 outage path renders the outage banner with retry-after", async () => {
    signedInTier2User();
    render(
      <StockAnalysisPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as AnalyzeStockOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
    expect(screen.getByText(/Retry available in 30s/)).toBeDefined();
  });

  test("404 symbol_not_found renders a helpful empty hint", async () => {
    signedInTier2User();
    render(
      <StockAnalysisPanel
        load={async () =>
          ({
            kind: "error",
            code: "symbol_not_found",
            message: "x",
            httpStatus: 404,
            retryAfterSecs: null,
          }) as AnalyzeStockOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="not-found"]')).not.toBeNull();
    });
  });

  test("generic error renders message + error-code data attribute", async () => {
    signedInTier2User();
    render(
      <StockAnalysisPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as AnalyzeStockOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot footer surfaces 'Showing cached snapshot.' hint", async () => {
    signedInTier2User();
    render(
      <StockAnalysisPanel load={readyLoader({ ...SAMPLE, stale: true })} />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });
});

describe("StockAnalysisPanel — registration + lifecycle", () => {
  test("registers itself with usePanelStore on mount", () => {
    signedInTier2User();
    render(<StockAnalysisPanel load={readyLoader(SAMPLE)} />);
    const layout = usePanelStore
      .getState()
      .getLayout(STOCK_ANALYSIS_PANEL_ID);
    expect(layout?.id).toBe(STOCK_ANALYSIS_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("hidden panels render nothing", () => {
    signedInTier2User();
    usePanelStore.getState().hide(STOCK_ANALYSIS_PANEL_ID);
    const { container } = render(
      <StockAnalysisPanel load={readyLoader(SAMPLE)} />,
    );
    expect(container.querySelector("[data-panel-id]")).toBeNull();
  });
});

describe("metricTiles", () => {
  test("returns 4 tiles in fixed order", () => {
    const tiles = metricTiles(SAMPLE);
    expect(tiles.map((t) => t.id)).toEqual([
      "price",
      "previous-close",
      "dollar-change",
      "percent-change",
    ]);
  });

  test("up trend → positive tone on the change tiles", () => {
    const tiles = metricTiles(SAMPLE);
    expect(tiles[2]?.tone).toBe("positive");
    expect(tiles[3]?.tone).toBe("positive");
  });

  test("down trend → negative tone on the change tiles", () => {
    const tiles = metricTiles({
      ...SAMPLE,
      metrics: { ...SAMPLE.metrics, trend: "down" },
    });
    expect(tiles[2]?.tone).toBe("negative");
    expect(tiles[3]?.tone).toBe("negative");
  });

  test("flat trend → neutral tone on the change tiles", () => {
    const tiles = metricTiles({
      ...SAMPLE,
      metrics: { ...SAMPLE.metrics, trend: "flat" },
    });
    expect(tiles[2]?.tone).toBe("neutral");
    expect(tiles[3]?.tone).toBe("neutral");
  });

  test("dollar-change tile formats with sign + currency", () => {
    const tiles = metricTiles(SAMPLE);
    expect(tiles[2]?.value).toBe("+2.00 USD");
  });

  test("percent-change tile renders subline 'trend · magnitude'", () => {
    const tiles = metricTiles(SAMPLE);
    expect(tiles[3]?.subline).toBe("up · small");
  });
});
