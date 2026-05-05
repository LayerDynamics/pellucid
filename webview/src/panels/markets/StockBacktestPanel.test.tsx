import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  StockBacktestPanel,
  STOCK_BACKTEST_PANEL_ID,
  REQUIRED_TIER,
  STRATEGY_ORDER,
} from "./StockBacktestPanel";
import type {
  BacktestStockOutcome,
  BacktestStockResponse,
} from "../../data/loaders/market/backtest-stock";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

function signedInTier2() {
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
      exportFormats: ["json", "csv"],
      validUntilMs: Date.now() + 60_000,
    },
  });
}

function signedInTier1() {
  useAuthStore.getState().signIn({
    userId: "u",
    email: "u@example.com",
    clerkSessionToken: "tok",
    entitlements: {
      tier: 1,
      maxDashboards: 1,
      apiAccess: false,
      apiRateLimit: 60,
      prioritySupport: false,
      exportFormats: ["json"],
      validUntilMs: Date.now() + 60_000,
    },
  });
}

const SAMPLE: BacktestStockResponse = {
  strategies: [
    {
      strategy: "equal-weight",
      picks: [
        { symbol: "SPY", weight: 0.5, percentChange: 0.5, contribution: 0.25 },
        { symbol: "QQQ", weight: 0.5, percentChange: -0.2, contribution: -0.1 },
      ],
      metrics: {
        totalReturnPct: 0.15,
        winRatePct: 50,
        maxDrawdownPct: -0.1,
      },
    },
    {
      strategy: "momentum",
      picks: [
        { symbol: "SPY", weight: 1, percentChange: 0.5, contribution: 0.5 },
        { symbol: "QQQ", weight: -1, percentChange: -0.2, contribution: 0.2 },
      ],
      metrics: { totalReturnPct: 0.7, winRatePct: 100, maxDrawdownPct: 0 },
    },
    {
      strategy: "mean-reversion",
      picks: [
        { symbol: "SPY", weight: -1, percentChange: 0.5, contribution: -0.5 },
        { symbol: "QQQ", weight: 1, percentChange: -0.2, contribution: -0.2 },
      ],
      metrics: {
        totalReturnPct: -0.7,
        winRatePct: 0,
        maxDrawdownPct: -0.5,
      },
    },
  ],
  universe: ["SPY", "QQQ"],
  stale: false,
  assembledAtMs: 1_700_000_000_000,
};

function readyLoader(response: BacktestStockResponse) {
  return async () => ({ kind: "ready", response }) as BacktestStockOutcome;
}

describe("StockBacktestPanel — view states", () => {
  test("loading → ready renders three strategy cards in plan order", async () => {
    signedInTier2();
    render(<StockBacktestPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(
        document.querySelectorAll('[data-component="StrategyCard"]').length,
      ).toBe(3);
    });
    const order = Array.from(
      document.querySelectorAll('[data-component="StrategyCard"]'),
    ).map((el) => el.getAttribute("data-strategy"));
    expect(order).toEqual(STRATEGY_ORDER);
  });

  test("renders the universe header with stale=false omitting the cached banner", async () => {
    signedInTier2();
    render(<StockBacktestPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="ready"]'),
      ).not.toBeNull();
    });
    expect(document.querySelector('[data-field="stale"]')).toBeNull();
  });

  test("stale=true renders the cached-snapshot footnote", async () => {
    signedInTier2();
    render(
      <StockBacktestPanel
        load={readyLoader({ ...SAMPLE, stale: true })}
      />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-field="stale"]'),
      ).not.toBeNull();
    });
  });

  test("503 outage path renders the outage banner with retry-after", async () => {
    signedInTier2();
    render(
      <StockBacktestPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "upstream is empty",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as BacktestStockOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="outage"]'),
      ).not.toBeNull();
    });
  });

  test("404 empty_universe renders the empty-universe banner", async () => {
    signedInTier2();
    render(
      <StockBacktestPanel
        load={async () =>
          ({
            kind: "error",
            code: "empty_universe",
            message: "filter excluded all rows",
            httpStatus: 404,
            retryAfterSecs: null,
          }) as BacktestStockOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="empty-universe"]'),
      ).not.toBeNull();
    });
  });

  test("generic error renders the error banner with code data attribute", async () => {
    signedInTier2();
    render(
      <StockBacktestPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as BacktestStockOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });
});

describe("StockBacktestPanel — registration + tier gate", () => {
  test("registers itself with usePanelStore on mount (tier-2 user)", () => {
    signedInTier2();
    render(<StockBacktestPanel load={readyLoader(SAMPLE)} />);
    const layout = usePanelStore.getState().getLayout(STOCK_BACKTEST_PANEL_ID);
    expect(layout).toBeDefined();
    expect(layout?.rowSpan).toBe(3);
    expect(layout?.colSpan).toBe(3);
  });

  test("hidden panels render nothing", () => {
    signedInTier2();
    usePanelStore.getState().hide(STOCK_BACKTEST_PANEL_ID);
    const { container } = render(<StockBacktestPanel load={readyLoader(SAMPLE)} />);
    expect(container.querySelector('[data-panel-id]')).toBeNull();
  });

  test("REQUIRED_TIER is 2 per the plan", () => {
    expect(REQUIRED_TIER).toBe(2);
  });

  test("tier-1 user sees the locked banner without invoking the loader", async () => {
    signedInTier1();
    let calls = 0;
    render(
      <StockBacktestPanel
        load={async () => {
          calls++;
          return { kind: "ready", response: SAMPLE } as BacktestStockOutcome;
        }}
      />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="locked"]'),
      ).not.toBeNull();
    });
    expect(calls).toBe(0);
  });

  test("anonymous user (tier 0) also sees the locked banner", async () => {
    render(<StockBacktestPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="locked"]'),
      ).not.toBeNull();
    });
  });
});

describe("StockBacktestPanel — universe controls", () => {
  test("editing the limit input refetches with the new ?limitUniverse", async () => {
    signedInTier2();
    const seen: Array<{ limitUniverse?: number; symbols?: string }> = [];
    const loader = async (q?: { limitUniverse?: number; symbols?: string }) => {
      seen.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as BacktestStockOutcome;
    };
    render(<StockBacktestPanel load={loader} />);
    await waitFor(() => seen.length === 1);
    const input = document.querySelector(
      'input[data-field="limit-input"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "20" } });
    await waitFor(() => seen.length >= 2);
    const last = seen[seen.length - 1];
    expect(last?.limitUniverse).toBe(20);
  });

  test("typing into the symbols input refetches with the trimmed CSV", async () => {
    signedInTier2();
    const seen: Array<{ limitUniverse?: number; symbols?: string }> = [];
    const loader = async (q?: { limitUniverse?: number; symbols?: string }) => {
      seen.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as BacktestStockOutcome;
    };
    render(<StockBacktestPanel load={loader} />);
    const input = document.querySelector(
      'input[data-field="symbols-input"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "  SPY,QQQ  " } });
    await waitFor(() => seen.length >= 2);
    const last = seen[seen.length - 1];
    expect(last?.symbols).toBe("SPY,QQQ");
  });

  test("clearing symbols input drops the field from the query", async () => {
    signedInTier2();
    const seen: Array<{ limitUniverse?: number; symbols?: string }> = [];
    const loader = async (q?: { limitUniverse?: number; symbols?: string }) => {
      seen.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as BacktestStockOutcome;
    };
    render(
      <StockBacktestPanel
        query={{ symbols: "SPY" }}
        load={loader}
      />,
    );
    const input = document.querySelector(
      'input[data-field="symbols-input"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "" } });
    await waitFor(() => seen.length >= 2);
    const last = seen[seen.length - 1];
    expect(last?.symbols).toBeUndefined();
  });
});
