import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  DAILY_MARKET_BRIEF_PANEL_ID,
  DailyMarketBriefPanel,
  REQUIRED_TIER,
  toneClass,
  formatAssembledAtUtc,
} from "./DailyMarketBriefPanel";
import type {
  DailyBriefOutcome,
  DailyBriefResponse,
} from "../../data/loaders/market/daily-brief";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const SAMPLE: DailyBriefResponse = {
  lines: [
    {
      section: "stocks",
      tone: "strong-up",
      headline: "Stocks broadly up",
      rationale: "Basket avg +0.60% across 8 symbols",
    },
    {
      section: "vix",
      tone: "up",
      headline: "Volatility low",
      rationale: "VIX at 14.0",
    },
    {
      section: "crypto",
      tone: "down",
      headline: "Crypto edging down",
      rationale: "Crypto basket 24h avg -0.30% across 8 tokens",
    },
  ],
  assembledAtMs: Date.UTC(2026, 4, 5, 11, 59, 0),
  stale: false,
};

const ready = (r: DailyBriefResponse) => async () =>
  ({ kind: "ready", response: r }) as DailyBriefOutcome;

describe("DailyMarketBriefPanel — view states", () => {
  test("ready renders one BriefLine per section", async () => {
    render(<DailyMarketBriefPanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const lines = document.querySelectorAll('[data-component="BriefLine"]');
    expect(lines.length).toBe(3);
  });

  test("each BriefLine surfaces tone + section data attrs", async () => {
    render(<DailyMarketBriefPanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const stocks = document.querySelector('[data-component="BriefLine"][data-section="stocks"]');
    const crypto = document.querySelector('[data-component="BriefLine"][data-section="crypto"]');
    expect(stocks?.getAttribute("data-tone")).toBe("strong-up");
    expect(crypto?.getAttribute("data-tone")).toBe("down");
  });

  test("each BriefLine renders the headline + rationale", async () => {
    render(<DailyMarketBriefPanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const stocks = document.querySelector('[data-component="BriefLine"][data-section="stocks"]');
    expect(stocks?.querySelector('[data-field="headline"]')?.textContent).toBe(
      "Stocks broadly up",
    );
    expect(stocks?.querySelector('[data-field="rationale"]')?.textContent).toContain(
      "+0.60%",
    );
  });

  test("empty lines render empty-state row", async () => {
    render(<DailyMarketBriefPanel load={ready({ ...SAMPLE, lines: [] })} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="empty"]')).not.toBeNull();
    });
  });

  test("503 outage renders outage banner", async () => {
    render(
      <DailyMarketBriefPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as DailyBriefOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("generic error renders error banner with code attr", async () => {
    render(
      <DailyMarketBriefPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as DailyBriefOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot footer renders cached hint", async () => {
    render(<DailyMarketBriefPanel load={ready({ ...SAMPLE, stale: true })} />);
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });
});

describe("DailyMarketBriefPanel — registration", () => {
  test("registers with usePanelStore", () => {
    render(<DailyMarketBriefPanel load={ready(SAMPLE)} />);
    const layout = usePanelStore.getState().getLayout(DAILY_MARKET_BRIEF_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("REQUIRED_TIER is 0", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("toneClass", () => {
  test("up-side tones use the success token", () => {
    expect(toneClass("strong-up")).toContain("success");
    expect(toneClass("up")).toContain("success");
  });
  test("down-side tones use the danger token", () => {
    expect(toneClass("strong-down")).toContain("danger");
    expect(toneClass("down")).toContain("danger");
  });
  test("mixed → fg", () => {
    expect(toneClass("mixed")).toContain("fg");
  });
});

describe("formatAssembledAtUtc", () => {
  test("HH:MM UTC", () => {
    expect(formatAssembledAtUtc(Date.UTC(2026, 4, 5, 1, 2, 0))).toBe("01:02 UTC");
  });
  test("non-finite or zero → em-dash", () => {
    expect(formatAssembledAtUtc(0)).toBe("—");
    expect(formatAssembledAtUtc(Number.NaN)).toBe("—");
  });
});
