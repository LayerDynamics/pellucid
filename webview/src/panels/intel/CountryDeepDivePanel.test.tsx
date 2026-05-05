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
  COUNTRY_DEEP_DIVE_PANEL_ID,
  CountryDeepDivePanel,
  DEFAULT_COUNTRY,
  REQUIRED_TIER,
  formatAssembledAt,
  parseGdeltSeenDate,
} from "./CountryDeepDivePanel";
import type {
  CountryDeepDiveOutcome,
  CountryDeepDiveResponse,
} from "../../data/loaders/intel/country-deep-dive";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const NOW_MS = Date.UTC(2026, 4, 5, 12, 0, 0);
const ASSEMBLED_AT_MS = Date.UTC(2026, 4, 5, 11, 59, 30);

const SAMPLE: CountryDeepDiveResponse = {
  country: "Iran",
  region: "Middle East",
  actors: [
    { name: "IRGC", events: 8, totalFatalities: 5 },
    { name: "Quds Force", events: 3, totalFatalities: 1 },
  ],
  articles: [
    {
      url: "https://a/1",
      title: "Title 1",
      domain: "a.com",
      language: "English",
      seenDate: "20260505T115500Z",
    },
  ],
  telegram: [
    {
      channel: "rt_intl_news",
      dataPost: "rt_intl_news/1",
      url: "https://t.me/rt_intl_news/1",
      datetime: "2026-05-05T11:50:00Z",
      text: "Iran update",
      views: "1.2K",
    },
  ],
  totals: { events: 8, incidents: 1, messages: 1 },
  assembledAtMs: ASSEMBLED_AT_MS,
  stale: false,
};

function readyLoader(response: CountryDeepDiveResponse) {
  return async () => ({ kind: "ready", response }) as CountryDeepDiveOutcome;
}

describe("CountryDeepDivePanel — view states", () => {
  test("loading → ready transitions and renders all three sections", async () => {
    render(
      <CountryDeepDivePanel load={readyLoader(SAMPLE)} now={NOW_MS} />,
    );
    expect(screen.getByText("Loading deep dive…")).toBeDefined();
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    expect(
      document.querySelector('[data-component="DeepDiveSection"][data-section="actors"]'),
    ).not.toBeNull();
    expect(
      document.querySelector('[data-component="DeepDiveSection"][data-section="articles"]'),
    ).not.toBeNull();
    expect(
      document.querySelector('[data-component="DeepDiveSection"][data-section="telegram"]'),
    ).not.toBeNull();
  });

  test("renders one row per actor + per article + per telegram message", async () => {
    render(
      <CountryDeepDivePanel load={readyLoader(SAMPLE)} now={NOW_MS} />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    expect(
      document.querySelectorAll('[data-component="DeepDiveActor"]').length,
    ).toBe(2);
    expect(
      document.querySelectorAll('[data-component="DeepDiveArticle"]').length,
    ).toBe(1);
    expect(
      document.querySelectorAll('[data-component="DeepDiveTelegram"]').length,
    ).toBe(1);
  });

  test("totals header shows N events · N incidents · N messages", async () => {
    render(
      <CountryDeepDivePanel load={readyLoader(SAMPLE)} now={NOW_MS} />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-component="DeepDiveTotals"]'),
      ).not.toBeNull();
    });
    const totals = document.querySelector(
      '[data-component="DeepDiveTotals"]',
    );
    expect(totals?.textContent).toContain("8 events");
    expect(totals?.textContent).toContain("1 incidents");
    expect(totals?.textContent).toContain("1 messages");
  });

  test("region + assembled-at footer surfaces metadata", async () => {
    render(
      <CountryDeepDivePanel load={readyLoader(SAMPLE)} now={NOW_MS} />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-component="DeepDiveFooter"]'),
      ).not.toBeNull();
    });
    const region = document.querySelector('[data-field="region"]');
    expect(region?.textContent).toContain("Middle East");
    const assembled = document.querySelector('[data-field="assembled-at"]');
    expect(assembled?.textContent).toContain("11:59:30 UTC");
  });

  test("empty response renders empty-state row", async () => {
    render(
      <CountryDeepDivePanel
        load={readyLoader({
          country: "Iran",
          region: "Middle East",
          actors: [],
          articles: [],
          telegram: [],
          totals: { events: 0, incidents: 0, messages: 0 },
          assembledAtMs: ASSEMBLED_AT_MS,
          stale: false,
        })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="empty"]')).not.toBeNull();
    });
  });

  test("503 outage path renders the outage banner", async () => {
    render(
      <CountryDeepDivePanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "x",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as CountryDeepDiveOutcome
        }
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="outage"]')).not.toBeNull();
    });
  });

  test("invalid_request error renders with error-code data attribute", async () => {
    render(
      <CountryDeepDivePanel
        load={async () =>
          ({
            kind: "error",
            code: "invalid_request",
            message: "missing",
            httpStatus: 400,
            retryAfterSecs: null,
          }) as CountryDeepDiveOutcome
        }
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("invalid_request");
    });
  });

  test("stale response shows the 'Showing cached snapshot.' hint", async () => {
    render(
      <CountryDeepDivePanel
        load={readyLoader({ ...SAMPLE, stale: true })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });
});

describe("CountryDeepDivePanel — country picker + presets", () => {
  test("typing in the picker triggers a refetch with the new country", async () => {
    const calls: Array<{ country: string }> = [];
    const loader = async (q: { country: string }) => {
      calls.push(q);
      return { kind: "ready", response: SAMPLE } as CountryDeepDiveOutcome;
    };
    render(<CountryDeepDivePanel load={loader} now={NOW_MS} />);
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

  test("clicking a preset selects that country", async () => {
    const calls: Array<{ country: string }> = [];
    const loader = async (q: { country: string }) => {
      calls.push(q);
      return { kind: "ready", response: SAMPLE } as CountryDeepDiveOutcome;
    };
    render(<CountryDeepDivePanel load={loader} now={NOW_MS} />);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(1));
    const btn = document.querySelector(
      'button[data-country-preset="Ukraine"]',
    ) as HTMLButtonElement;
    fireEvent.click(btn);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected a call");
    expect(last.country).toBe("Ukraine");
  });

  test("default country is `DEFAULT_COUNTRY` when no prop is set", async () => {
    const calls: Array<{ country: string }> = [];
    const loader = async (q: { country: string }) => {
      calls.push(q);
      return { kind: "ready", response: SAMPLE } as CountryDeepDiveOutcome;
    };
    render(<CountryDeepDivePanel load={loader} now={NOW_MS} />);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(1));
    const first = calls[0];
    if (!first) throw new Error("expected a call");
    expect(first.country).toBe(DEFAULT_COUNTRY);
  });
});

describe("CountryDeepDivePanel — registration + lifecycle", () => {
  test("registers itself with usePanelStore on mount", () => {
    render(
      <CountryDeepDivePanel load={readyLoader(SAMPLE)} now={NOW_MS} />,
    );
    const layout = usePanelStore
      .getState()
      .getLayout(COUNTRY_DEEP_DIVE_PANEL_ID);
    expect(layout?.id).toBe(COUNTRY_DEEP_DIVE_PANEL_ID);
    expect(layout?.rowSpan).toBe(3);
    expect(layout?.colSpan).toBe(4);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(COUNTRY_DEEP_DIVE_PANEL_ID);
    const { container } = render(
      <CountryDeepDivePanel load={readyLoader(SAMPLE)} now={NOW_MS} />,
    );
    expect(container.querySelector("[data-panel-id]")).toBeNull();
  });

  test("REQUIRED_TIER is 0 (anonymous)", () => {
    expect(REQUIRED_TIER).toBe(0);
  });
});

describe("parseGdeltSeenDate / formatAssembledAt", () => {
  test("parses YYYYMMDDTHHMMSSZ into UTC ms", () => {
    expect(parseGdeltSeenDate("20260504T120000Z")).toBe(
      Date.UTC(2026, 4, 4, 12, 0, 0),
    );
  });

  test("returns null on malformed input", () => {
    expect(parseGdeltSeenDate("nope")).toBeNull();
  });

  test("formatAssembledAt formats wall-clock ms as HH:MM:SS UTC", () => {
    expect(formatAssembledAt(Date.UTC(2026, 4, 5, 1, 2, 3))).toBe("01:02:03 UTC");
  });

  test("formatAssembledAt returns em-dash for non-finite or zero", () => {
    expect(formatAssembledAt(0)).toBe("—");
    expect(formatAssembledAt(Number.NaN)).toBe("—");
  });
});
