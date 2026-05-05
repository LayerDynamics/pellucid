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
  TELEGRAM_INTEL_PANEL_ID,
  TelegramIntelPanel,
  REQUIRED_TIER,
  DEFAULT_LIMIT,
} from "./TelegramIntelPanel";
import type {
  FeedOutcome,
  FeedResponse,
  TelegramMessage,
} from "../../data/loaders/intel/telegram";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
});

const NOW_MS = Date.UTC(2026, 4, 5, 12, 0, 0);
const ASSEMBLED_AT_MS = Date.UTC(2026, 4, 5, 11, 59, 30);

function message(channel: string, id: number): TelegramMessage {
  return {
    channel,
    dataPost: `${channel}/${id}`,
    url: `https://t.me/${channel}/${id}`,
    datetime: "2026-05-05T11:55:00Z",
    text: `body-${channel}-${id}`,
    views: "1.2K",
  };
}

const SAMPLE: FeedResponse = {
  rows: [message("rt_intl_news", 1), message("isw_warstudies", 2)],
  channels: ["rt_intl_news", "isw_warstudies"],
  assembledAtMs: ASSEMBLED_AT_MS,
  total: 2,
  stale: false,
};

function readyLoader(response: FeedResponse) {
  return async () => ({ kind: "ready", response }) as FeedOutcome;
}

describe("TelegramIntelPanel — view states", () => {
  test("loading → ready transitions and renders one row per message", async () => {
    render(<TelegramIntelPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    expect(screen.getByText("Loading Telegram intel…")).toBeDefined();
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const rows = document.querySelectorAll('[data-component="TelegramRow"]');
    expect(rows.length).toBe(2);
  });

  test("renders channel basket from response.channels", async () => {
    render(<TelegramIntelPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const chips = document.querySelectorAll("[data-channel-chip]");
    expect(chips.length).toBe(2);
    const labels = Array.from(chips).map((el) =>
      el.getAttribute("data-channel-chip"),
    );
    expect(labels).toEqual(["rt_intl_news", "isw_warstudies"]);
  });

  test("renders views chip with the upstream-rendered counter", async () => {
    render(<TelegramIntelPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    await waitFor(() => {
      expect(document.querySelector('[data-state="ready"]')).not.toBeNull();
    });
    const viewsEls = document.querySelectorAll('[data-field="views"]');
    expect(viewsEls.length).toBe(2);
    expect(viewsEls[0]?.textContent).toContain("1.2K");
  });

  test("empty rows → empty-state row", async () => {
    render(
      <TelegramIntelPanel
        load={readyLoader({
          rows: [],
          channels: [],
          assembledAtMs: ASSEMBLED_AT_MS,
          total: 0,
          stale: false,
        })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-state="empty"]')).not.toBeNull();
    });
  });

  test("503 outage path renders the outage banner with retry-after countdown", async () => {
    render(
      <TelegramIntelPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "upstream is empty",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as FeedOutcome
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
      <TelegramIntelPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as FeedOutcome
        }
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("stale snapshot shows the 'Showing cached snapshot.' hint", async () => {
    render(
      <TelegramIntelPanel
        load={readyLoader({ ...SAMPLE, stale: true })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(document.querySelector('[data-field="stale"]')).not.toBeNull();
    });
  });

  test("assembled-at footer renders an HH:MM:SS UTC timestamp", async () => {
    render(<TelegramIntelPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    await waitFor(() => {
      const el = document.querySelector('[data-field="assembled-at"]');
      expect(el?.textContent).toContain("11:59:30 UTC");
    });
  });
});

describe("TelegramIntelPanel — channel basket filter", () => {
  test("clicking a channel chip refetches with channel query", async () => {
    const calls: Array<{ channel?: string; limit?: number }> = [];
    const loader = async (q?: { channel?: string; limit?: number }) => {
      calls.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as FeedOutcome;
    };
    render(<TelegramIntelPanel load={loader} now={NOW_MS} />);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(1));
    await waitFor(() => {
      expect(document.querySelector("[data-channel-chip]")).not.toBeNull();
    });
    const chip = document.querySelector(
      'button[data-channel-chip="isw_warstudies"]',
    ) as HTMLButtonElement;
    fireEvent.click(chip);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected a call");
    expect(last.channel).toBe("isw_warstudies");
  });

  test("clicking the same chip twice toggles back to no filter", async () => {
    const calls: Array<{ channel?: string; limit?: number }> = [];
    const loader = async (q?: { channel?: string; limit?: number }) => {
      calls.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as FeedOutcome;
    };
    render(<TelegramIntelPanel load={loader} now={NOW_MS} />);
    await waitFor(() => {
      expect(document.querySelector("[data-channel-chip]")).not.toBeNull();
    });
    const chipSelector = 'button[data-channel-chip="rt_intl_news"]';
    const chip1 = document.querySelector(chipSelector) as HTMLButtonElement;
    fireEvent.click(chip1);
    // Wait for the first toggle (chip ON) to be observable before
    // clicking again so React 18 batching doesn't collapse both
    // clicks against the same initial null state. Re-query after
    // every state transition because React may have replaced the
    // node during the re-render.
    await waitFor(() => {
      const c = document.querySelector(chipSelector) as HTMLButtonElement;
      expect(c.getAttribute("aria-pressed")).toBe("true");
    });
    const chip2 = document.querySelector(chipSelector) as HTMLButtonElement;
    fireEvent.click(chip2);
    await waitFor(() => {
      const c = document.querySelector(chipSelector) as HTMLButtonElement;
      expect(c.getAttribute("aria-pressed")).toBe("false");
    });
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected a call");
    expect(last.channel).toBeUndefined();
  });

  test("'Clear filter' button restores the unfiltered query", async () => {
    const calls: Array<{ channel?: string; limit?: number }> = [];
    const loader = async (q?: { channel?: string; limit?: number }) => {
      calls.push(q ?? {});
      return { kind: "ready", response: SAMPLE } as FeedOutcome;
    };
    render(
      <TelegramIntelPanel
        load={loader}
        now={NOW_MS}
        query={{ channel: "rt_intl_news" }}
      />,
    );
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(1));
    await waitFor(() => {
      expect(
        document.querySelector('button[data-action="clear-channel"]'),
      ).not.toBeNull();
    });
    const clear = document.querySelector(
      'button[data-action="clear-channel"]',
    ) as HTMLButtonElement;
    fireEvent.click(clear);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected a call");
    expect(last.channel).toBeUndefined();
  });
});

describe("TelegramIntelPanel — registration + lifecycle", () => {
  test("registers itself with usePanelStore on mount", () => {
    render(<TelegramIntelPanel load={readyLoader(SAMPLE)} now={NOW_MS} />);
    const layout = usePanelStore
      .getState()
      .getLayout(TELEGRAM_INTEL_PANEL_ID);
    expect(layout?.id).toBe(TELEGRAM_INTEL_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(TELEGRAM_INTEL_PANEL_ID);
    const { container } = render(
      <TelegramIntelPanel load={readyLoader(SAMPLE)} now={NOW_MS} />,
    );
    expect(container.querySelector("[data-panel-id]")).toBeNull();
  });

  test("DEFAULT_LIMIT is 50, REQUIRED_TIER is 0", () => {
    expect(DEFAULT_LIMIT).toBe(50);
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
