import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen, waitFor, fireEvent } from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import { useNewsStore } from "../../state/useNewsStore";
import {
  NewsPanel,
  NEWS_PANEL_ID,
  REQUIRED_TIER,
} from "./NewsPanel";
import type {
  ListArticlesOutcome,
  ListArticlesResponse,
} from "../../data/loaders/news/list";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
  useNewsStore.getState().clear();
});

const NOW_MS = Date.UTC(2026, 4, 4, 12, 0, 0);

const SAMPLE_RESPONSE: ListArticlesResponse = {
  articles: [
    {
      id: "a1",
      title: "Suez convoy resumes",
      source: "Reuters",
      publishedAtMs: NOW_MS - 2 * 60 * 60 * 1000,
      url: "https://example.com/a1",
      severity: "high",
    },
    {
      id: "a2",
      title: "BoJ holds rates",
      source: "Nikkei",
      publishedAtMs: NOW_MS - 6 * 60 * 60 * 1000,
      severity: "info",
    },
    {
      id: "a3",
      title: "Wildfire alert",
      source: "AP",
      publishedAtMs: NOW_MS - 30 * 60 * 1000,
      severity: "critical",
    },
  ],
  total: 3,
  stale: false,
};

function readyLoader(response: ListArticlesResponse) {
  return async () =>
    ({ kind: "ready", response } as ListArticlesOutcome);
}

function delayedLoader(ms: number, outcome: ListArticlesOutcome) {
  return () =>
    new Promise<ListArticlesOutcome>((resolve) =>
      setTimeout(() => resolve(outcome), ms),
    );
}

describe("NewsPanel — view states", () => {
  test("loading → ready transitions and renders one card per article", async () => {
    render(<NewsPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />);
    // Loading state initially.
    expect(screen.getByText("Loading news…")).toBeDefined();
    // Becomes ready.
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="ready"]'),
      ).not.toBeNull();
    });
    const cards = document.querySelectorAll('[data-component="NewsCard"]');
    expect(cards.length).toBe(3);
  });

  test("empty article list renders an empty-state row, not the cards block", async () => {
    render(
      <NewsPanel
        load={readyLoader({ articles: [], total: 0, stale: false })}
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="empty"]'),
      ).not.toBeNull();
    });
    expect(document.querySelector('[data-state="ready"]')).toBeNull();
  });

  test("503 outage path renders the outage banner with retry-after countdown", async () => {
    const loader = readyLoader as unknown as () => Promise<ListArticlesOutcome>;
    void loader;
    render(
      <NewsPanel
        load={async () =>
          ({
            kind: "error",
            code: "bootstrap_upstream_empty",
            message: "upstream is empty",
            httpStatus: 503,
            retryAfterSecs: 30,
          }) as ListArticlesOutcome
        }
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="outage"]'),
      ).not.toBeNull();
    });
    expect(screen.getByText(/Retry available in 30s/)).toBeDefined();
  });

  test("generic error renders a message + error code data attribute", async () => {
    render(
      <NewsPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "db locked",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as ListArticlesOutcome
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

  test("loading state shows briefly when loader is async", async () => {
    render(
      <NewsPanel
        load={delayedLoader(20, {
          kind: "ready",
          response: SAMPLE_RESPONSE,
        })}
        now={NOW_MS}
      />,
    );
    expect(screen.getByText("Loading news…")).toBeDefined();
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="ready"]'),
      ).not.toBeNull();
    });
  });
});

describe("NewsPanel — registration + store side-effects", () => {
  test("registers itself with usePanelStore on mount", () => {
    render(<NewsPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />);
    const layout = usePanelStore.getState().getLayout(NEWS_PANEL_ID);
    expect(layout).toBeDefined();
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
    expect(layout?.id).toBe(NEWS_PANEL_ID);
  });

  test("hidden panels render nothing", () => {
    usePanelStore.getState().hide(NEWS_PANEL_ID);
    const { container } = render(
      <NewsPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />,
    );
    expect(container.querySelector('[data-panel-id]')).toBeNull();
  });

  test("mirrors response into useNewsStore.feed on success", async () => {
    render(<NewsPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />);
    await waitFor(() => {
      expect(useNewsStore.getState().feed.length).toBe(3);
    });
    expect(useNewsStore.getState().feed.map((a) => a.id)).toEqual([
      "a1",
      "a2",
      "a3",
    ]);
  });

  test("does NOT touch useNewsStore.feed on outcome=error", async () => {
    useNewsStore.getState().setFeed([
      {
        id: "preexisting",
        title: "x",
        source: "x",
        publishedAtMs: 0,
      },
    ]);
    render(
      <NewsPanel
        load={async () =>
          ({
            kind: "error",
            code: "cache_failure",
            message: "err",
            httpStatus: 502,
            retryAfterSecs: null,
          }) as ListArticlesOutcome
        }
        now={NOW_MS}
      />,
    );
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="error"]'),
      ).not.toBeNull();
    });
    // Pre-existing feed must NOT be cleared by the error path.
    expect(
      useNewsStore.getState().feed.map((a) => a.id),
    ).toEqual(["preexisting"]);
  });
});

describe("NewsPanel — severity floor toolbar", () => {
  test("renders 'All' + 4 severity chips", () => {
    render(<NewsPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />);
    const tb = screen.getByRole("toolbar", { name: "Severity floor" });
    expect(tb).toBeDefined();
    const chips = tb.querySelectorAll("button");
    // 1 "All" + 4 severities.
    expect(chips.length).toBe(5);
  });

  test("clicking 'critical' triggers a refetch with severity=critical", async () => {
    const calls: Array<{ severity?: string; limit?: number }> = [];
    const loader = async (q?: { severity?: string; limit?: number }) => {
      calls.push(q ?? {});
      return {
        kind: "ready",
        response: SAMPLE_RESPONSE,
      } as ListArticlesOutcome;
    };
    render(<NewsPanel load={loader} now={NOW_MS} />);
    await waitFor(() => calls.length === 1);
    const btn = document.querySelector(
      'button[data-severity-floor="critical"]',
    ) as HTMLButtonElement;
    fireEvent.click(btn);
    await waitFor(() => calls.length === 2);
    expect(calls.length).toBeGreaterThanOrEqual(2);
    // Latest call carries the floor.
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected at least one loader call");
    expect(last.severity).toBe("critical");
  });

  test("clicking the same severity twice toggles back to All (no severity)", async () => {
    const calls: Array<{ severity?: string; limit?: number }> = [];
    const loader = async (q?: { severity?: string; limit?: number }) => {
      calls.push(q ?? {});
      return {
        kind: "ready",
        response: SAMPLE_RESPONSE,
      } as ListArticlesOutcome;
    };
    render(<NewsPanel load={loader} now={NOW_MS} />);
    const btn = document.querySelector(
      'button[data-severity-floor="warn"]',
    ) as HTMLButtonElement;
    fireEvent.click(btn);
    fireEvent.click(btn);
    await waitFor(() => calls.length >= 3);
    const last = calls[calls.length - 1];
    if (!last) throw new Error("expected at least one loader call");
    expect(last.severity).toBeUndefined();
  });

  test("'All' button is aria-pressed by default", () => {
    render(<NewsPanel load={readyLoader(SAMPLE_RESPONSE)} now={NOW_MS} />);
    const all = document.querySelector(
      'button[data-severity-floor="all"]',
    ) as HTMLButtonElement;
    expect(all.getAttribute("aria-pressed")).toBe("true");
  });
});

describe("NewsPanel — locked tier path", () => {
  test("REQUIRED_TIER is 0 (anonymous) per the plan", () => {
    // Tier-0 means everyone passes the gate; the locked branch is
    // unreachable through the normal entitlement flow. The branch
    // exists for forward-compat (a future task may bump
    // REQUIRED_TIER to 1+) — covered separately by the
    // `LiveNewsPanel` tier-1 lock test which exercises the same
    // branch via the real entitlement store without monkey-
    // patching `hasTier`.
    expect(REQUIRED_TIER).toBe(0);
  });
});
