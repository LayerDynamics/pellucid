import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  STABLECOIN_PANEL_ID,
  StablecoinPanel,
  REQUIRED_TIER,
  formatBillions,
  formatSignedPercent,
  formatAssembledAtUtc,
} from "./StablecoinPanel";
import type {
  StablecoinsOutcome,
  StablecoinsResponse,
} from "../../data/loaders/market/stablecoins";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const SAMPLE: StablecoinsResponse = {
  rows: [
    {
      id: "tether",
      symbol: "USDT",
      usd: 1.0001,
      usd24hChange: 0.01,
      usdMarketCap: 100_000_000_000,
      pegDeviationPct: 0.01,
      depegRisk: false,
      lastUpdatedAt: 1_714_060_800,
    },
    {
      id: "dai",
      symbol: "DAI",
      usd: 0.985,
      usd24hChange: -0.5,
      usdMarketCap: 5_000_000_000,
      pegDeviationPct: -1.5,
      depegRisk: true,
      lastUpdatedAt: 1_714_060_800,
    },
  ],
  totalMarketCapUsd: 105_000_000_000,
  depegCount: 1,
  assembledAtMs: Date.UTC(2026, 4, 5, 11, 59, 0),
  stale: false,
};

const ready = (r: StablecoinsResponse) => async () =>
  ({ kind: "ready", response: r }) as StablecoinsOutcome;

describe("StablecoinPanel — view states", () => {
  test("ready renders rows + summary", async () => {
    render(<StablecoinPanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const rows = document.querySelectorAll('[data-component="StablecoinRow"]');
    expect(rows.length).toBe(2);
    expect(document.querySelector('[data-field="market-cap"]')?.textContent).toContain(
      "$105.00B",
    );
    expect(document.querySelector('[data-field="depeg-count"]')?.textContent).toContain(
      "1",
    );
  });

  test("depeg row gets danger tone + badge", async () => {
    render(<StablecoinPanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const dai = document.querySelector('[data-component="StablecoinRow"][data-symbol="DAI"]');
    expect(dai?.getAttribute("data-tone")).toBe("danger");
    expect(dai?.querySelector('[data-field="depeg-badge"]')).not.toBeNull();
    const usdt = document.querySelector('[data-component="StablecoinRow"][data-symbol="USDT"]');
    expect(usdt?.getAttribute("data-tone")).toBe("neutral");
    expect(usdt?.querySelector('[data-field="depeg-badge"]')).toBeNull();
  });

  test("empty rows render empty-state row", async () => {
    render(
      <StablecoinPanel
        load={ready({ ...SAMPLE, rows: [], totalMarketCapUsd: 0, depegCount: 0 })}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="empty"]')).not.toBeNull();
    });
  });

  test("503 outage renders outage banner", async () => {
    render(
      <StablecoinPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as StablecoinsOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("generic error renders error banner with code attr", async () => {
    render(
      <StablecoinPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as StablecoinsOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot footer renders cached hint", async () => {
    render(<StablecoinPanel load={ready({ ...SAMPLE, stale: true })} />);
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });
});

describe("StablecoinPanel — registration", () => {
  test("registers with usePanelStore", () => {
    render(<StablecoinPanel load={ready(SAMPLE)} />);
    const layout = usePanelStore.getState().getLayout(STABLECOIN_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("REQUIRED_TIER is 0", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("formatBillions / formatSignedPercent / formatAssembledAtUtc", () => {
  test("formatBillions billions / millions / sub-million", () => {
    expect(formatBillions(2_500_000_000)).toBe("$2.50B");
    expect(formatBillions(750_000_000)).toBe("$750.00M");
    expect(formatBillions(500)).toBe("$500");
    expect(formatBillions(0)).toBe("—");
    expect(formatBillions(Number.NaN)).toBe("—");
  });
  test("formatSignedPercent signed", () => {
    expect(formatSignedPercent(0.5)).toBe("+0.50%");
    expect(formatSignedPercent(-0.4)).toBe("-0.40%");
    expect(formatSignedPercent(0)).toBe("0.00%");
    expect(formatSignedPercent(Number.NaN)).toBe("—");
  });
  test("formatAssembledAtUtc HH:MM UTC", () => {
    expect(formatAssembledAtUtc(Date.UTC(2026, 4, 5, 1, 2, 0))).toBe("01:02 UTC");
    expect(formatAssembledAtUtc(0)).toBe("—");
  });
});
