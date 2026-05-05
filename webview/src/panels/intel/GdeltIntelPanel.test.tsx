import { afterEach, describe, expect, test } from "bun:test";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import {
  DEFAULT_LIMIT,
  formatAssembledAt,
  GDELT_INTEL_PANEL_ID,
  GdeltIntelPanel,
  REQUIRED_TIER,
} from "./GdeltIntelPanel";
import type {
  GdeltFeedOutcome,
  GdeltFeedResponse,
} from "../../data/loaders/intel/gdelt";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const NOW_MS = Date.UTC(2026, 4, 5, 12, 0, 0);
const ASSEMBLED_AT_MS = Date.UTC(2026, 4, 5, 11, 59, 30);

const SAMPLE_RESPONSE: GdeltFeedResponse = {
  rows: [
    {
      url: "https://a.example/1",
      title: "Reuters: Title 1",
      seenDate: "20260505T115500Z",
      socialImage: "",
      domain: "a.example",
      language: "English",
      sourceCountry: "Iran",
    },
    {
      url: "https://b.example/2",
      title: "Title 2",
      seenDate: "20260505T115000Z",
      socialImage: "",
      domain: "b.example",
      language: "Persian",
      sourceCountry: "Iran",
    },
  ],
  query: "(theme:KILL)",
  timespan: "24h",
  assembledAtMs: ASSEMBLED_AT_MS,
  total: 2,
  stale: false,
};

function readyLoader(response: GdeltFeedResponse) {
  return async () => ({ kind: "ready", response }) as GdeltFeedOutcome;
}

function delayedLoader(ms: number, outcome: GdeltFeedOutcome) {
  return () =>
    new Promise<GdeltFeedOutcome>((resolve) =>
      setTimeout(() => resolve(outcome), ms),
    );
}

describe("GdeltIntelPanel — view states", () => {
  test("loading → ready transitions and renders one row per article", async () => {
    render(
      <GdeltIntelPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />,
    );
    expect(screen.getByText("Loading GDELT intel…")).toBeDefined();
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const rows = document.querySelectorAll('[data-component="GdeltRow"]');
    expect(rows.length).toBe(2);
  });

  test("renders country + language entity chips per row", async () => {
    render(
      <GdeltIntelPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const chips = document.querySelectorAll(
      '[data-component="IntelEntityChip"]',
    );
    // 2 rows × (country + language) = 4 chips.
    expect(chips.length).toBe(4);
    const kinds = Array.from(chips).map((el) =>
      el.getAttribute("data-entity-kind"),
    );
    expect(kinds.filter((k) => k === "country").length).toBe(2);
    expect(kinds.filter((k) => k === "topic").length).toBe(2);
  });

  test("empty rows → empty-state row, not the ready block", async () => {
    render(
      <GdeltIntelPanel
        load={readyLoader({
          rows: [],
          total: 0,
          query: "x",
          timespan: "24h",
          assembledAtMs: ASSEMBLED_AT_MS,
          stale: false,
        })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="empty"]')).not.toBeNull();
    });
    expect(document.querySelector('[data-state="ready"]')).toBeNull();
  });

  test("503 outage path renders the outage banner with retry-after countdown", async () => {
    render(
      <GdeltIntelPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "upstream is empty",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as GdeltFeedOutcome
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
      <GdeltIntelPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as GdeltFeedOutcome
        }
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
    expect(screen.getByText(/Cache failure/)).toBeDefined();
    expect(screen.getByText("db locked")).toBeDefined();
  });

  test("stale snapshot footer surfaces the 'Showing cached snapshot.' hint", async () => {
    render(
      <GdeltIntelPanel
        load={readyLoader({ ...SAMPLE_RESPONSE, stale: true })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });

  test("assembled-at footer renders an HH:MM:SS UTC timestamp", async () => {
    render(
      <GdeltIntelPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-field="assembled-at"]');
      expect(el?.textContent).toContain("11:59:30 UTC");
    });
  });

  test("loading state shows briefly when loader is async", async () => {
    render(
      <GdeltIntelPanel
        load={delayedLoader(20, { kind: "ready", response: SAMPLE_RESPONSE })}
        now={NOW_MS}
      />,
    );
    expect(screen.getByText("Loading GDELT intel…")).toBeDefined();
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
  });
});

describe("GdeltIntelPanel — country filter", () => {
  test("typing a country name triggers a refetch with country query", async () => {
    const calls: Array<{ country?: string; limit?: number }> = [];
    const loader = async (q?: { country?: string; limit?: number }) => {
      calls.push(q ?? {});
      return { kind: "ready", response: SAMPLE_RESPONSE } as GdeltFeedOutcome;
    };
    render(<GdeltIntelPanel load={loader} now={NOW_MS} />);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(1));
    const input = document.querySelector(
      'input[aria-label="Country filter"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "Iran" } });
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected at least one call");
    expect(last.country).toBe("Iran");
  });

  test("clearing the input removes the country query", async () => {
    const calls: Array<{ country?: string; limit?: number }> = [];
    const loader = async (q?: { country?: string; limit?: number }) => {
      calls.push(q ?? {});
      return { kind: "ready", response: SAMPLE_RESPONSE } as GdeltFeedOutcome;
    };
    render(
      <GdeltIntelPanel
        load={loader}
        now={NOW_MS}
        query={{ country: "Iran" }}
      />,
    );
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(1));
    const input = document.querySelector(
      'input[aria-label="Country filter"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "" } });
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected at least one call");
    expect(last.country).toBeUndefined();
  });

  test("whitespace-only input does not set the country query", async () => {
    const calls: Array<{ country?: string }> = [];
    const loader = async (q?: { country?: string }) => {
      calls.push(q ?? {});
      return { kind: "ready", response: SAMPLE_RESPONSE } as GdeltFeedOutcome;
    };
    render(<GdeltIntelPanel load={loader} now={NOW_MS} />);
    const input = document.querySelector(
      'input[aria-label="Country filter"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "   " } });
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(1));
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected at least one call");
    expect(last.country).toBeUndefined();
  });
});

describe("GdeltIntelPanel — registration + lifecycle", () => {
  test("registers itself with usePanelStore on mount", () => {
    render(
      <GdeltIntelPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />,
    );
    const layout = usePanelStore.getState().getLayout(GDELT_INTEL_PANEL_ID);
    expect(layout?.id).toBe(GDELT_INTEL_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(GDELT_INTEL_PANEL_ID);
    const { container } = render(
      <GdeltIntelPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />,
    );
    expect(container.querySelector("[data-panel-id]")).toBeNull();
  });

  test("DEFAULT_LIMIT is 50", () => {
    expect(DEFAULT_LIMIT).toBe(50);
  });

  test("REQUIRED_TIER is 0 (anonymous)", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("formatAssembledAt", () => {
  test("formats a wall-clock ms value as HH:MM:SS UTC", () => {
    expect(formatAssembledAt(Date.UTC(2026, 4, 5, 1, 2, 3))).toBe(
      "01:02:03 UTC",
    );
  });

  test("returns em-dash for non-finite or zero input", () => {
    expect(formatAssembledAt(0)).toBe("—");
    expect(formatAssembledAt(Number.NaN)).toBe("—");
    expect(formatAssembledAt(-1)).toBe("—");
  });
});
