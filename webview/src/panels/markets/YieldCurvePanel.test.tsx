import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  YIELD_CURVE_PANEL_ID,
  YieldCurvePanel,
  REQUIRED_TIER,
  formatSpread,
  formatAssembledAtUtc,
} from "./YieldCurvePanel";
import type {
  YieldCurveOutcome,
  YieldCurveResponse,
} from "../../data/loaders/market/yield-curve";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const SAMPLE: YieldCurveResponse = {
  points: [
    { seriesCode: "DGS3MO", maturityLabel: "3M", maturityMonths: 3, yieldPct: 5.0, observationDate: "2026-05-05" },
    { seriesCode: "DGS2", maturityLabel: "2Y", maturityMonths: 24, yieldPct: 4.5, observationDate: "2026-05-05" },
    { seriesCode: "DGS5", maturityLabel: "5Y", maturityMonths: 60, yieldPct: 4.2, observationDate: "2026-05-05" },
    { seriesCode: "DGS10", maturityLabel: "10Y", maturityMonths: 120, yieldPct: 4.1, observationDate: "2026-05-05" },
    { seriesCode: "DGS30", maturityLabel: "30Y", maturityMonths: 360, yieldPct: 4.4, observationDate: "2026-05-05" },
  ],
  spreads: { tenMinusTwo: -0.4, tenMinusThreeMonth: -0.9, thirtyMinusFive: 0.2 },
  inverted: true,
  assembledAtMs: Date.UTC(2026, 4, 5, 11, 59, 0),
  stale: false,
};

const ready = (r: YieldCurveResponse) => async () =>
  ({ kind: "ready", response: r }) as YieldCurveOutcome;

describe("YieldCurvePanel — view states", () => {
  test("ready renders sparkline + spread strip + points table", async () => {
    render(<YieldCurvePanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    expect(document.querySelector('[data-component="YieldSparkline"]')).not.toBeNull();
    expect(document.querySelector('[data-component="SpreadStrip"]')).not.toBeNull();
    const rows = document.querySelectorAll('[data-component="YieldPointRow"]');
    expect(rows.length).toBe(5);
  });

  test("inverted curve surfaces the inverted badge", async () => {
    render(<YieldCurvePanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-field="inverted-badge"]')).not.toBeNull();
    });
    const strip = document.querySelector('[data-component="SpreadStrip"]');
    expect(strip?.getAttribute("data-inverted")).toBe("true");
  });

  test("non-inverted curve omits the inverted badge", async () => {
    render(
      <YieldCurvePanel
        load={ready({
          ...SAMPLE,
          inverted: false,
          spreads: { tenMinusTwo: 0.5, tenMinusThreeMonth: 0.4, thirtyMinusFive: 0.2 },
        })}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    expect(document.querySelector('[data-field="inverted-badge"]')).toBeNull();
  });

  test("503 outage renders outage banner", async () => {
    render(
      <YieldCurvePanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as YieldCurveOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("generic error renders error banner with code attr", async () => {
    render(
      <YieldCurvePanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as YieldCurveOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot footer renders cached hint", async () => {
    render(<YieldCurvePanel load={ready({ ...SAMPLE, stale: true })} />);
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });
});

describe("YieldCurvePanel — registration", () => {
  test("registers with usePanelStore", () => {
    render(<YieldCurvePanel load={ready(SAMPLE)} />);
    const layout = usePanelStore.getState().getLayout(YIELD_CURVE_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(3);
  });

  test("REQUIRED_TIER is 0", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("formatSpread / formatAssembledAtUtc", () => {
  test("formatSpread signed", () => {
    expect(formatSpread(0.5)).toBe("+0.50%");
    expect(formatSpread(-0.4)).toBe("-0.40%");
    expect(formatSpread(undefined)).toBe("—");
    expect(formatSpread(Number.NaN)).toBe("—");
  });
  test("formatAssembledAtUtc HH:MM UTC", () => {
    expect(formatAssembledAtUtc(Date.UTC(2026, 4, 5, 1, 2, 0))).toBe("01:02 UTC");
    expect(formatAssembledAtUtc(0)).toBe("—");
  });
});
