import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  COT_POSITIONING_PANEL_ID,
  CotPositioningPanel,
  DEFAULT_SORT,
  REQUIRED_TIER,
  formatAssembledAtUtc,
  formatSignedPercent,
  formatSignedThousands,
  formatThousands,
  netTone,
} from "./CotPositioningPanel";
import type { CotOutcome, CotResponse } from "../../data/loaders/market/cot";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const ASSEMBLED_AT_MS = Date.UTC(2026, 4, 5, 11, 59, 0);

const SAMPLE: CotResponse = {
  rows: [
    {
      contractCode: "088691",
      contractName: "GOLD",
      reportDate: "2026-04-29",
      openInterestAll: 480_000,
      producerLong: 100_000,
      producerShort: 120_000,
      swapLong: 80_000,
      swapShort: 90_000,
      managedMoneyLong: 120_000,
      managedMoneyShort: 60_000,
      managedMoneyNet: 60_000,
      managedMoneyNetPctOi: 12.5,
    },
    {
      contractCode: "067411",
      contractName: "CRUDE OIL",
      reportDate: "2026-04-29",
      openInterestAll: 1_200_000,
      producerLong: 200_000,
      producerShort: 350_000,
      swapLong: 150_000,
      swapShort: 100_000,
      managedMoneyLong: 80_000,
      managedMoneyShort: 200_000,
      managedMoneyNet: -120_000,
      managedMoneyNetPctOi: -10.0,
    },
  ],
  total: 2,
  assembledAtMs: ASSEMBLED_AT_MS,
  stale: false,
};

function readyLoader(response: CotResponse) {
  return async () => ({ kind: "ready", response }) as CotOutcome;
}

describe("CotPositioningPanel — view states", () => {
  test("loading → ready transitions and renders one row per contract", async () => {
    render(<CotPositioningPanel load={readyLoader(SAMPLE)} />);
    expect(screen.getByText("Loading positioning…")).toBeDefined();
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const rows = document.querySelectorAll('[data-component="CotTableRow"]');
    expect(rows.length).toBe(2);
  });

  test("managed-money-net cell tone reflects sign", async () => {
    render(<CotPositioningPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const gold = document.querySelector(
      '[data-component="CotTableRow"][data-contract-code="088691"]',
    );
    const oil = document.querySelector(
      '[data-component="CotTableRow"][data-contract-code="067411"]',
    );
    expect(gold?.getAttribute("data-tone")).toBe("positive");
    expect(oil?.getAttribute("data-tone")).toBe("negative");
  });

  test("managed-money-net + %OI cells render the signed values", async () => {
    render(<CotPositioningPanel load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const gold = document.querySelector(
      '[data-component="CotTableRow"][data-contract-code="088691"]',
    );
    const net = gold?.querySelector('[data-field="managed-money-net"]');
    const pct = gold?.querySelector('[data-field="managed-money-pct-oi"]');
    expect(net?.textContent).toBe("+60,000");
    expect(pct?.textContent).toBe("+12.5%");
  });

  test("empty rows → empty-state row, not the ready block", async () => {
    render(
      <CotPositioningPanel
        load={readyLoader({ ...SAMPLE, rows: [], total: 0 })}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="empty"]')).not.toBeNull();
    });
  });

  test("503 outage path renders the outage banner with retry-after", async () => {
    render(
      <CotPositioningPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as CotOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("400 invalid_request renders error banner with code", async () => {
    render(
      <CotPositioningPanel
        load={async () =>
          ({
            kind: "error",
            code: "invalid_request",
            message: "bad sort",
            httpStatus: 400,
            retryAfterSecs: null,
          }) as CotOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("invalid_request");
    });
  });

  test("stale snapshot footer surfaces the cached-snapshot hint", async () => {
    render(
      <CotPositioningPanel load={readyLoader({ ...SAMPLE, stale: true })} />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });
});

describe("CotPositioningPanel — sort control", () => {
  test("changing the sort dropdown refetches with the new ?sort", async () => {
    const seen: Array<{ sort?: string }> = [];
    const loader = async (q?: { sort?: string }) => {
      seen.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as CotOutcome;
    };
    render(<CotPositioningPanel load={loader} />);
    await waitFor(() => expect(seen.length).toBeGreaterThanOrEqual(1));
    const select = document.querySelector(
      'select[data-field="sort-select"]',
    ) as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "open-interest-desc" } });
    await waitFor(() => expect(seen.length).toBeGreaterThanOrEqual(2));
    const last = seen[seen.length - 1];
    if (!last) throw new Error("expected a call");
    expect(last.sort).toBe("open-interest-desc");
  });

  test("first call uses DEFAULT_SORT", async () => {
    const seen: Array<{ sort?: string }> = [];
    const loader = async (q?: { sort?: string }) => {
      seen.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as CotOutcome;
    };
    render(<CotPositioningPanel load={loader} />);
    await waitFor(() => expect(seen.length).toBeGreaterThanOrEqual(1));
    const first = seen[0];
    if (!first) throw new Error("expected a call");
    expect(first.sort).toBe(DEFAULT_SORT);
  });
});

describe("CotPositioningPanel — registration + lifecycle", () => {
  test("registers itself with usePanelStore on mount", () => {
    render(<CotPositioningPanel load={readyLoader(SAMPLE)} />);
    const layout = usePanelStore.getState().getLayout(COT_POSITIONING_PANEL_ID);
    expect(layout?.id).toBe(COT_POSITIONING_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(3);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(COT_POSITIONING_PANEL_ID);
    const { container } = render(
      <CotPositioningPanel load={readyLoader(SAMPLE)} />,
    );
    expect(container.querySelector("[data-panel-id]")).toBeNull();
  });

  test("REQUIRED_TIER is 0 (anonymous)", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("netTone", () => {
  test("positive / negative / zero / non-finite", () => {
    expect(netTone(5_000)).toBe("positive");
    expect(netTone(-5_000)).toBe("negative");
    expect(netTone(0)).toBe("neutral");
    expect(netTone(Number.NaN)).toBe("neutral");
  });
});

describe("formatThousands / formatSignedThousands / formatSignedPercent", () => {
  test("formatThousands groups by comma", () => {
    expect(formatThousands(1_234_567)).toBe("1,234,567");
    expect(formatThousands(0)).toBe("0");
    expect(formatThousands(Number.NaN)).toBe("—");
  });

  test("formatSignedThousands prepends sign + groups", () => {
    expect(formatSignedThousands(60_000)).toBe("+60,000");
    expect(formatSignedThousands(-60_000)).toBe("-60,000");
    expect(formatSignedThousands(0)).toBe("0");
    expect(formatSignedThousands(Number.NaN)).toBe("—");
  });

  test("formatSignedPercent uses 1 decimal + sign", () => {
    expect(formatSignedPercent(12.5)).toBe("+12.5%");
    expect(formatSignedPercent(-3.2)).toBe("-3.2%");
    expect(formatSignedPercent(0)).toBe("0.0%");
    expect(formatSignedPercent(Number.NaN)).toBe("—");
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
