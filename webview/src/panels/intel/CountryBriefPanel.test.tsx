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
  COUNTRY_BRIEF_PANEL_ID,
  CountryBriefPanel,
  DEFAULT_COUNTRY,
  REQUIRED_TIER,
  formatAssembledAt,
  formatRelative,
  parseGdeltSeenDate,
} from "./CountryBriefPanel";
import type {
  CountryBriefOutcome,
  CountryBriefResponse,
} from "../../data/loaders/intel/country-brief";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const NOW_MS = Date.UTC(2026, 4, 5, 12, 0, 0);
const ASSEMBLED_AT_MS = Date.UTC(2026, 4, 5, 11, 59, 30);

const SAMPLE: CountryBriefResponse = {
  country: "Iran",
  region: "Middle East",
  summary: "8 ACLED events, 1 GDELT incident, 0 Telegram mentions.",
  topActor: { name: "IRGC", events: 8 },
  topArticle: {
    url: "https://a/1",
    title: "freshest",
    domain: "a.com",
    seenDate: "20260505T115500Z",
  },
  totals: { events: 8, incidents: 1, messages: 0 },
  assembledAtMs: ASSEMBLED_AT_MS,
  stale: false,
};

function readyLoader(response: CountryBriefResponse) {
  return async () => ({ kind: "ready", response }) as CountryBriefOutcome;
}

describe("CountryBriefPanel — view states", () => {
  test("loading → ready transitions and renders summary + counts + top sections", async () => {
    render(<CountryBriefPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    expect(screen.getByText("Loading brief…")).toBeDefined();
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const summary = document.querySelector('[data-field="summary"]');
    expect(summary?.textContent).toContain("8 ACLED events");
    expect(
      document.querySelector('[data-component="BriefCounts"]'),
    ).not.toBeNull();
    expect(
      document.querySelector('[data-component="BriefTopActor"]'),
    ).not.toBeNull();
    expect(
      document.querySelector('[data-component="BriefTopArticle"]'),
    ).not.toBeNull();
  });

  test("counts strip renders NE / NI / NM", async () => {
    render(<CountryBriefPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    await waitFor(() => {
      expect(
        document.querySelector('[data-component="BriefCounts"]'),
      ).not.toBeNull();
    });
    const counts = document.querySelector('[data-component="BriefCounts"]');
    expect(counts?.querySelector('[data-field="counts-events"]')?.textContent).toBe(
      "8E",
    );
    expect(
      counts?.querySelector('[data-field="counts-incidents"]')?.textContent,
    ).toBe("1I");
    expect(
      counts?.querySelector('[data-field="counts-messages"]')?.textContent,
    ).toBe("0M");
  });

  test("top-actor section omitted when topActor is null", async () => {
    render(
      <CountryBriefPanel
        load={readyLoader({ ...SAMPLE, topActor: null })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    expect(
      document.querySelector('[data-component="BriefTopActor"]'),
    ).toBeNull();
  });

  test("top-article section omitted when topArticle is null", async () => {
    render(
      <CountryBriefPanel
        load={readyLoader({ ...SAMPLE, topArticle: null })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    expect(
      document.querySelector('[data-component="BriefTopArticle"]'),
    ).toBeNull();
  });

  test("503 outage path renders the outage banner", async () => {
    render(
      <CountryBriefPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as CountryBriefOutcome
        }
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("error path renders message + error-code data attribute", async () => {
    render(
      <CountryBriefPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as CountryBriefOutcome
        }
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot footer renders the 'cached' hint", async () => {
    render(
      <CountryBriefPanel
        load={readyLoader({ ...SAMPLE, stale: true })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });

  test("assembled-at footer surfaces an HH:MM:SS UTC timestamp", async () => {
    render(<CountryBriefPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    await waitFor(() => {
      const el = document.querySelector('[data-field="assembled-at"]');
      expect(el?.textContent).toContain("11:59:30 UTC");
    });
  });
});

describe("CountryBriefPanel — country picker", () => {
  test("typing triggers a refetch with the new country", async () => {
    const calls: Array<{ country: string }> = [];
    const loader = async (q: { country: string }) => {
      calls.push(q);
      return { kind: "ready", response: SAMPLE } as CountryBriefOutcome;
    };
    render(<CountryBriefPanel load={loader} now={NOW_MS} />);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(1));
    const input = document.querySelector(
      'input[aria-label="Country"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "Israel" } });
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected a call");
    expect(last.country).toBe("Israel");
  });

  test("default country is `DEFAULT_COUNTRY` when no prop is set", async () => {
    const calls: Array<{ country: string }> = [];
    const loader = async (q: { country: string }) => {
      calls.push(q);
      return { kind: "ready", response: SAMPLE } as CountryBriefOutcome;
    };
    render(<CountryBriefPanel load={loader} now={NOW_MS} />);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(1));
    const first = calls[0];
    if (!first) throw new Error("expected a call");
    expect(first.country).toBe(DEFAULT_COUNTRY);
  });
});

describe("CountryBriefPanel — registration + lifecycle", () => {
  test("registers itself with usePanelStore on mount with 1x1 layout", () => {
    render(<CountryBriefPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    const layout = usePanelStore.getState().getLayout(COUNTRY_BRIEF_PANEL_ID);
    expect(layout?.id).toBe(COUNTRY_BRIEF_PANEL_ID);
    expect(layout?.rowSpan).toBe(1);
    expect(layout?.colSpan).toBe(1);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(COUNTRY_BRIEF_PANEL_ID);
    const { container } = render(
      <CountryBriefPanel load={readyLoader(SAMPLE)} now={NOW_MS} />,
    );
    expect(container.querySelector("[data-panel-id]")).toBeNull();
  });

  test("REQUIRED_TIER is 0", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("CountryBriefPanel — helpers", () => {
  test("parseGdeltSeenDate parses YYYYMMDDTHHMMSSZ into UTC ms", () => {
    expect(parseGdeltSeenDate("20260504T120000Z")).toBe(
      Date.UTC(2026, 4, 4, 12, 0, 0),
    );
    expect(parseGdeltSeenDate("nope")).toBeNull();
  });

  test("formatRelative covers seconds / minutes / hours / days", () => {
    const now = Date.UTC(2026, 4, 5, 12, 0, 0);
    expect(formatRelative(now - 30_000, now)).toBe("30s ago");
    expect(formatRelative(now - 5 * 60_000, now)).toBe("5m ago");
    expect(formatRelative(now - 3 * 3_600_000, now)).toBe("3h ago");
    expect(formatRelative(now - 2 * 86_400_000, now)).toBe("2d ago");
    expect(formatRelative(now + 1_000, now)).toBe("just now");
  });

  test("formatAssembledAt formats HH:MM:SS UTC and handles bad inputs", () => {
    expect(formatAssembledAt(Date.UTC(2026, 4, 5, 1, 2, 3))).toBe("01:02:03 UTC");
    expect(formatAssembledAt(0)).toBe("—");
    expect(formatAssembledAt(Number.NaN)).toBe("—");
  });
});
