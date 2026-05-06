import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  LIQUIDITY_SHIFTS_PANEL_ID,
  LiquidityShiftsPanel,
  REQUIRED_TIER,
  deltaTone,
  formatBillions,
  formatSignedDelta,
  formatSignedPercent,
  formatAssembledAtUtc,
} from "./LiquidityShiftsPanel";
import type {
  LiquidityShiftsOutcome,
  LiquidityShiftsResponse,
} from "../../data/loaders/market/liquidity-shifts";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const SAMPLE: LiquidityShiftsResponse = {
  series: [
    {
      seriesCode: "WALCL",
      latestValue: 7_200,
      latestDate: "2026-05-01",
      priorValue: 7_180,
      periodDelta: 20,
      periodDeltaPct: 0.279,
    },
    {
      seriesCode: "M2SL",
      latestValue: 21_000,
      latestDate: "2026-05-01",
      priorValue: 20_980,
      periodDelta: 20,
      periodDeltaPct: 0.095,
    },
    {
      seriesCode: "RRPONTSYD",
      latestValue: 450,
      latestDate: "2026-05-01",
      priorValue: 480,
      periodDelta: -30,
      periodDeltaPct: -6.25,
    },
  ],
  netLiquidityBillionUsd: 6_750,
  assembledAtMs: Date.UTC(2026, 4, 5, 11, 59, 0),
  stale: false,
};

const ready = (r: LiquidityShiftsResponse) => async () =>
  ({ kind: "ready", response: r }) as LiquidityShiftsOutcome;

describe("LiquidityShiftsPanel — view states", () => {
  test("ready renders net-liquidity hero + series rows", async () => {
    render(<LiquidityShiftsPanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    expect(
      document.querySelector('[data-field="net-liquidity"]')?.textContent,
    ).toBe("$6.75T");
    const rows = document.querySelectorAll('[data-component="LiquidityRow"]');
    expect(rows.length).toBe(3);
  });

  test("series rows tone-coded by period delta sign", async () => {
    render(<LiquidityShiftsPanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const walcl = document.querySelector(
      '[data-component="LiquidityRow"][data-series-code="WALCL"]',
    );
    const rrp = document.querySelector(
      '[data-component="LiquidityRow"][data-series-code="RRPONTSYD"]',
    );
    expect(walcl?.getAttribute("data-tone")).toBe("positive");
    expect(rrp?.getAttribute("data-tone")).toBe("negative");
  });

  test("503 outage renders outage banner", async () => {
    render(
      <LiquidityShiftsPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as LiquidityShiftsOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("generic error renders error banner with code attr", async () => {
    render(
      <LiquidityShiftsPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as LiquidityShiftsOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot footer renders cached hint", async () => {
    render(<LiquidityShiftsPanel load={ready({ ...SAMPLE, stale: true })} />);
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });
});

describe("LiquidityShiftsPanel — registration", () => {
  test("registers with usePanelStore", () => {
    render(<LiquidityShiftsPanel load={ready(SAMPLE)} />);
    const layout = usePanelStore.getState().getLayout(LIQUIDITY_SHIFTS_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("REQUIRED_TIER is 0", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("deltaTone / formatBillions / formatSignedDelta / formatSignedPercent", () => {
  test("deltaTone classifies positive / negative / neutral", () => {
    expect(deltaTone(10)).toBe("positive");
    expect(deltaTone(-10)).toBe("negative");
    expect(deltaTone(0)).toBe("neutral");
    expect(deltaTone(undefined)).toBe("neutral");
    expect(deltaTone(Number.NaN)).toBe("neutral");
  });
  test("formatBillions trillions / billions / zero / NaN", () => {
    expect(formatBillions(7_200)).toBe("$7.20T");
    expect(formatBillions(450)).toBe("$450B");
    expect(formatBillions(0)).toBe("$0B");
    expect(formatBillions(Number.NaN)).toBe("—");
  });
  test("formatSignedDelta signed", () => {
    expect(formatSignedDelta(20)).toBe("+$20B");
    expect(formatSignedDelta(-30)).toBe("-$30B");
    expect(formatSignedDelta(0)).toBe("$0B");
    expect(formatSignedDelta(undefined)).toBe("—");
  });
  test("formatSignedPercent signed", () => {
    expect(formatSignedPercent(0.5)).toBe("+0.50%");
    expect(formatSignedPercent(-1.0)).toBe("-1.00%");
    expect(formatSignedPercent(0)).toBe("0.00%");
    expect(formatSignedPercent(Number.NaN)).toBe("—");
  });
  test("formatAssembledAtUtc HH:MM UTC", () => {
    expect(formatAssembledAtUtc(Date.UTC(2026, 4, 5, 1, 2, 0))).toBe("01:02 UTC");
    expect(formatAssembledAtUtc(0)).toBe("—");
  });
});
