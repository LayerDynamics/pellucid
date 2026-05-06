import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  EARNINGS_CALENDAR_PANEL_ID,
  EarningsCalendarPanel,
  REQUIRED_TIER,
  formatAssembledAtUtc,
  timingLabel,
} from "./EarningsCalendarPanel";
import type {
  EarningsOutcome,
  EarningsResponse,
} from "../../data/loaders/market/earnings";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const SAMPLE: EarningsResponse = {
  days: [
    {
      date: "2026-05-06",
      events: [
        {
          symbol: "SPY",
          company: "S&P 500 ETF",
          date: "2026-05-06",
          timing: "before-open",
          epsEstimate: 1.23,
        },
        {
          symbol: "DIA",
          company: "Dow ETF",
          date: "2026-05-06",
          timing: "after-close",
        },
      ],
    },
  ],
  total: 2,
  lookaheadDays: 7,
  assembledAtMs: Date.UTC(2026, 4, 5, 11, 59, 0),
  stale: false,
};

const ready = (r: EarningsResponse) => async () =>
  ({ kind: "ready", response: r }) as EarningsOutcome;

describe("EarningsCalendarPanel — view states", () => {
  test("ready renders one day group with events", async () => {
    render(<EarningsCalendarPanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    expect(
      document.querySelectorAll('[data-component="EarningsDayGroup"]').length,
    ).toBe(1);
    expect(
      document.querySelectorAll('[data-component="EarningsRow"]').length,
    ).toBe(2);
  });

  test("event with EPS estimate renders the est badge", async () => {
    render(<EarningsCalendarPanel load={ready(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const row = document.querySelector(
      '[data-component="EarningsRow"][data-symbol="SPY"]',
    );
    expect(row?.querySelector('[data-field="eps-estimate"]')?.textContent).toContain(
      "est $1.23",
    );
  });

  test("empty days renders empty-state row", async () => {
    render(
      <EarningsCalendarPanel load={ready({ ...SAMPLE, days: [], total: 0 })} />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="empty"]')).not.toBeNull();
    });
  });

  test("503 outage renders outage banner", async () => {
    render(
      <EarningsCalendarPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as EarningsOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("generic error renders error banner with code attr", async () => {
    render(
      <EarningsCalendarPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as EarningsOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot footer renders cached hint", async () => {
    render(<EarningsCalendarPanel load={ready({ ...SAMPLE, stale: true })} />);
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });
});

describe("EarningsCalendarPanel — registration", () => {
  test("registers with usePanelStore on mount", () => {
    render(<EarningsCalendarPanel load={ready(SAMPLE)} />);
    const layout = usePanelStore.getState().getLayout(EARNINGS_CALENDAR_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(3);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(EARNINGS_CALENDAR_PANEL_ID);
    const { container } = render(<EarningsCalendarPanel load={ready(SAMPLE)} />);
    expect(container.querySelector("[data-panel-id]")).toBeNull();
  });

  test("REQUIRED_TIER is 0", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("timingLabel", () => {
  test("maps every timing", () => {
    expect(timingLabel("before-open")).toBe("BMO");
    expect(timingLabel("after-close")).toBe("AMC");
    expect(timingLabel("during-hours")).toBe("INT");
    expect(timingLabel("unknown")).toBe("—");
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
