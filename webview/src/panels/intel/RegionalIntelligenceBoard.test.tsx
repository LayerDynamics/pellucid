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
  formatAssembledAt,
  REGIONAL_BOARD_PANEL_ID,
  RegionalIntelligenceBoard,
  REQUIRED_TIER,
} from "./RegionalIntelligenceBoard";
import type {
  RegionalOutcome,
  RegionalResponse,
} from "../../data/loaders/intel/regional";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const ASSEMBLED_AT_MS = Date.UTC(2026, 4, 5, 11, 59, 30);

const SAMPLE: RegionalResponse = {
  regions: [
    {
      region: "Middle East",
      totalEvents: 8,
      totalIncidents: 3,
      countries: [
        { name: "Iran", events: 3, incidents: 2 },
        { name: "Israel", events: 5, incidents: 1 },
      ],
      topActors: ["Hamas", "IDF"],
    },
    {
      region: "Europe",
      totalEvents: 4,
      totalIncidents: 1,
      countries: [{ name: "Ukraine", events: 4, incidents: 1 }],
      topActors: ["Russian Forces"],
    },
  ],
  assembledAtMs: ASSEMBLED_AT_MS,
  stale: false,
};

function readyLoader(response: RegionalResponse) {
  return async () => ({ kind: "ready", response }) as RegionalOutcome;
}

describe("RegionalIntelligenceBoard — view states", () => {
  test("loading → ready transitions and renders one card per region", async () => {
    render(<RegionalIntelligenceBoard load={readyLoader(SAMPLE)} />);
    expect(screen.getByText("Loading regional intel…")).toBeDefined();
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const cards = document.querySelectorAll('[data-component="RegionCard"]');
    expect(cards.length).toBe(2);
  });

  test("renders top-actor chips per region", async () => {
    render(<RegionalIntelligenceBoard load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const chips = document.querySelectorAll(
      '[data-component="IntelEntityChip"][data-entity-kind="actor"]',
    );
    // 2 in Middle East + 1 in Europe = 3.
    expect(chips.length).toBe(3);
  });

  test("renders per-country rollup rows with E/I counts", async () => {
    render(<RegionalIntelligenceBoard load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const rows = document.querySelectorAll(
      '[data-component="CountryRollup"]',
    );
    // ME has 2 + EU has 1 = 3.
    expect(rows.length).toBe(3);
    const iran = document.querySelector(
      '[data-component="CountryRollup"][data-country="Iran"]',
    );
    expect(iran?.textContent).toContain("3E");
    expect(iran?.textContent).toContain("2I");
  });

  test("totals chip on the card header reads NE · NI", async () => {
    render(<RegionalIntelligenceBoard load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const me = document.querySelector(
      '[data-component="RegionCard"][data-region="Middle East"]',
    );
    const totals = me?.querySelector('[data-field="totals"]');
    expect(totals?.textContent).toContain("8E");
    expect(totals?.textContent).toContain("3I");
  });

  test("empty regions → empty-state row", async () => {
    render(
      <RegionalIntelligenceBoard
        load={readyLoader({
          regions: [],
          assembledAtMs: ASSEMBLED_AT_MS,
          stale: false,
        })}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="empty"]')).not.toBeNull();
    });
  });

  test("503 outage path renders the outage banner with retry-after countdown", async () => {
    render(
      <RegionalIntelligenceBoard
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as RegionalOutcome
        }
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
    expect(screen.getByText(/Retry available in 30s/)).toBeDefined();
  });

  test("generic error renders message + error-code data attribute", async () => {
    render(
      <RegionalIntelligenceBoard
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as RegionalOutcome
        }
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot footer surfaces the 'Showing cached snapshot.' hint", async () => {
    render(
      <RegionalIntelligenceBoard
        load={readyLoader({ ...SAMPLE, stale: true })}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });

  test("assembled-at footer renders an HH:MM:SS UTC timestamp", async () => {
    render(<RegionalIntelligenceBoard load={readyLoader(SAMPLE)} />);
    await waitFor(() => {
      const el = document.querySelector('[data-field="assembled-at"]');
      expect(el?.textContent).toContain("11:59:30 UTC");
    });
  });
});

describe("RegionalIntelligenceBoard — region drill-in", () => {
  test("clicking a region name refetches with that region in the query", async () => {
    const calls: Array<{ region?: string }> = [];
    const loader = async (q?: { region?: string }) => {
      calls.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as RegionalOutcome;
    };
    render(<RegionalIntelligenceBoard load={loader} />);
    await waitFor(() => {
      expect(
        document.querySelector(
          '[data-component="RegionCard"][data-region="Middle East"]',
        ),
      ).not.toBeNull();
    });
    const btn = document.querySelector(
      '[data-component="RegionCard"][data-region="Middle East"] button[data-field="region-name"]',
    ) as HTMLButtonElement;
    fireEvent.click(btn);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected a call");
    expect(last.region).toBe("Middle East");
  });

  test("'Show all regions' clears the active region", async () => {
    const calls: Array<{ region?: string }> = [];
    const loader = async (q?: { region?: string }) => {
      calls.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as RegionalOutcome;
    };
    render(
      <RegionalIntelligenceBoard
        load={loader}
        query={{ region: "Middle East" }}
      />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('button[data-action="clear-region"]'),
      ).not.toBeNull();
    });
    const clear = document.querySelector(
      'button[data-action="clear-region"]',
    ) as HTMLButtonElement;
    fireEvent.click(clear);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected a call");
    expect(last.region).toBeUndefined();
  });
});

describe("RegionalIntelligenceBoard — registration + lifecycle", () => {
  test("registers itself with usePanelStore on mount with full-width board layout", () => {
    render(<RegionalIntelligenceBoard load={readyLoader(SAMPLE)} />);
    const layout = usePanelStore
      .getState()
      .getLayout(REGIONAL_BOARD_PANEL_ID);
    expect(layout?.id).toBe(REGIONAL_BOARD_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(4);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(REGIONAL_BOARD_PANEL_ID);
    const { container } = render(
      <RegionalIntelligenceBoard load={readyLoader(SAMPLE)} />,
    );
    expect(container.querySelector("[data-panel-id]")).toBeNull();
  });

  test("REQUIRED_TIER is 0 (anonymous)", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("formatAssembledAt", () => {
  test("formats wall-clock ms as HH:MM:SS UTC", () => {
    expect(formatAssembledAt(Date.UTC(2026, 4, 5, 1, 2, 3))).toBe("01:02:03 UTC");
  });

  test("returns em-dash for non-finite or zero input", () => {
    expect(formatAssembledAt(0)).toBe("—");
    expect(formatAssembledAt(Number.NaN)).toBe("—");
  });
});
