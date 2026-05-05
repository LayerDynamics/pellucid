import { afterEach, describe, expect, test } from "bun:test";
import {
  cleanup,
  fireEvent,
  render,
  waitFor,
} from "@testing-library/react";

import { useAuthStore } from "../../state/useAuthStore";
import { usePanelStore } from "../../state/usePanelStore";
import { useNewsStore } from "../../state/useNewsStore";
import {
  LiveNewsPanel,
  LIVE_NEWS_PANEL_ID,
  MAX_BUFFER,
  REQUIRED_TIER,
} from "./LiveNewsPanel";
import type { NewsArticle } from "../../data/loaders/news/list";
import type {
  LiveError,
  LiveSubscriberCallbacks,
  LiveSubscription,
  ReadyPayload,
} from "../../data/loaders/news/live";

afterEach(() => {
  cleanup();
  useAuthStore.getState().signOut();
  usePanelStore.getState().reset();
  useNewsStore.getState().clear();
});

const NOW_MS = Date.UTC(2026, 4, 5, 12, 0, 0);

function signedInTier1User() {
  useAuthStore.getState().signIn({
    userId: "u",
    email: "u@example.com",
    clerkSessionToken: "tok",
    entitlements: {
      tier: 1,
      maxDashboards: 1,
      apiAccess: false,
      apiRateLimit: 60,
      prioritySupport: false,
      exportFormats: ["json"],
      validUntilMs: Date.now() + 60_000,
    },
  });
}

function article(id: string, severity?: NewsArticle["severity"]): NewsArticle {
  const base: NewsArticle = {
    id,
    title: `title-${id}`,
    source: "src",
    publishedAtMs: NOW_MS - 60_000,
  };
  if (severity) base.severity = severity;
  return base;
}

interface FakeSub {
  callbacks: LiveSubscriberCallbacks;
  closed: boolean;
  closeCount: number;
  url: string;
}

function fakeSubscribeFactory() {
  const subs: FakeSub[] = [];
  const subscribe: typeof import("../../data/loaders/news/live").subscribe = (
    callbacks,
    query,
  ) => {
    const url = query?.severity
      ? `/api/news/v1/list-live?severity=${query.severity}`
      : "/api/news/v1/list-live";
    const sub: FakeSub = { callbacks, closed: false, closeCount: 0, url };
    subs.push(sub);
    const handle: LiveSubscription = {
      url,
      close: () => {
        sub.closed = true;
        sub.closeCount++;
      },
    };
    return handle;
  };
  return { subscribe, subs };
}

describe("LiveNewsPanel — view states", () => {
  test("connecting → open after onReady fires", async () => {
    signedInTier1User();
    const { subscribe, subs } = fakeSubscribeFactory();
    const { container } = render(
      <LiveNewsPanel subscribe={subscribe} now={NOW_MS} />,
    );
    expect(container.querySelector('[data-state="waiting"]')).not.toBeNull();
    expect(
      document
        .querySelector('[data-component="ConnectionDot"]')
        ?.getAttribute("data-connection-state"),
    ).toBe("connecting");
    expect(subs.length).toBe(1);
    const sub = subs[0];
    if (!sub) throw new Error("expected one subscription");
    sub.callbacks.onReady?.({
      articles: [article("a1"), article("a2")],
      total: 2,
      stale: false,
    } as ReadyPayload);
    await waitFor(() => {
      expect(
        document.querySelector('[data-state="streaming"]'),
      ).not.toBeNull();
    });
    expect(
      document
        .querySelector('[data-component="ConnectionDot"]')
        ?.getAttribute("data-connection-state"),
    ).toBe("open");
    const cards = container.querySelectorAll('[data-component="NewsCard"]');
    expect(cards.length).toBe(2);
  });

  test("article event prepends to the rolling list (latest first)", async () => {
    signedInTier1User();
    const { subscribe, subs } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    const sub = subs[0];
    if (!sub) throw new Error("expected one subscription");
    sub.callbacks.onReady?.({
      articles: [article("a1"), article("a2")],
      total: 2,
      stale: false,
    } as ReadyPayload);
    sub.callbacks.onArticle?.(article("a3"));
    sub.callbacks.onArticle?.(article("a4"));
    await waitFor(() => {
      expect(
        document.querySelectorAll('[data-component="NewsCard"]').length,
      ).toBe(4);
    });
    const ids = Array.from(
      document.querySelectorAll('[data-component="NewsCard"]'),
    ).map((el) => el.getAttribute("data-news-id"));
    // Latest first: a4, a3, a1, a2.
    expect(ids).toEqual(["a4", "a3", "a1", "a2"]);
  });

  test("duplicate article id replaces in place rather than duplicating", async () => {
    signedInTier1User();
    const { subscribe, subs } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    const sub = subs[0];
    if (!sub) throw new Error("expected one subscription");
    sub.callbacks.onReady?.({
      articles: [article("a1")],
      total: 1,
      stale: false,
    } as ReadyPayload);
    sub.callbacks.onArticle?.(article("a1", "critical")); // same id, new severity
    await waitFor(() => {
      const cards = document.querySelectorAll('[data-component="NewsCard"]');
      expect(cards.length).toBe(1);
    });
  });

  test("MAX_BUFFER is documented as a positive cap (sanity)", () => {
    // Sanity check on the constant — keeps the contract visible
    // and prevents accidental zero/negative bumps.
    expect(MAX_BUFFER).toBeGreaterThan(0);
    expect(MAX_BUFFER).toBeLessThanOrEqual(1000);
  });

  test("outage event renders the outage banner with reason data attribute", async () => {
    signedInTier1User();
    const { subscribe, subs } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    const sub = subs[0];
    if (!sub) throw new Error("expected one subscription");
    sub.callbacks.onOutage?.("bootstrap_upstream_empty");
    await waitFor(() => {
      const el = document.querySelector('[data-state="outage"]');
      expect(el?.getAttribute("data-outage-reason")).toBe(
        "bootstrap_upstream_empty",
      );
    });
  });

  test("error event renders the error banner with code data attribute", async () => {
    signedInTier1User();
    const { subscribe, subs } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    const sub = subs[0];
    if (!sub) throw new Error("expected one subscription");
    const err: LiveError = { code: "cache_failure", message: "db locked" };
    sub.callbacks.onError?.(err);
    await waitFor(() => {
      const el = document.querySelector('[data-state="error"]');
      expect(el?.getAttribute("data-error-code")).toBe("cache_failure");
    });
  });

  test("connection-error returns to 'connecting' without dropping articles", async () => {
    signedInTier1User();
    const { subscribe, subs } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    const sub = subs[0];
    if (!sub) throw new Error("expected one subscription");
    sub.callbacks.onReady?.({
      articles: [article("a1")],
      total: 1,
      stale: false,
    } as ReadyPayload);
    sub.callbacks.onConnectionError?.(new Event("error"));
    await waitFor(() => {
      expect(
        document
          .querySelector('[data-component="ConnectionDot"]')
          ?.getAttribute("data-connection-state"),
      ).toBe("connecting");
    });
    // The accumulated article must NOT have been wiped.
    expect(
      document.querySelectorAll('[data-component="NewsCard"]').length,
    ).toBe(1);
  });
});

describe("LiveNewsPanel — registration + lifecycle", () => {
  test("registers itself with usePanelStore on mount", () => {
    signedInTier1User();
    const { subscribe } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    const layout = usePanelStore.getState().getLayout(LIVE_NEWS_PANEL_ID);
    expect(layout?.id).toBe(LIVE_NEWS_PANEL_ID);
    expect(layout?.rowSpan).toBe(2);
    expect(layout?.colSpan).toBe(2);
  });

  test("hidden panels render nothing", () => {
    signedInTier1User();
    usePanelStore.getState().hide(LIVE_NEWS_PANEL_ID);
    const { subscribe } = fakeSubscribeFactory();
    const { container } = render(
      <LiveNewsPanel subscribe={subscribe} now={NOW_MS} />,
    );
    expect(container.querySelector('[data-panel-id]')).toBeNull();
  });

  test("unmount closes the underlying subscription", () => {
    signedInTier1User();
    const { subscribe, subs } = fakeSubscribeFactory();
    const { unmount } = render(
      <LiveNewsPanel subscribe={subscribe} now={NOW_MS} />,
    );
    const sub = subs[0];
    if (!sub) throw new Error("expected one subscription");
    expect(sub.closed).toBe(false);
    unmount();
    expect(sub.closed).toBe(true);
    expect(sub.closeCount).toBe(1);
  });

  test("mirrors articles into useNewsStore.feed on ready + delta", async () => {
    signedInTier1User();
    const { subscribe, subs } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    const sub = subs[0];
    if (!sub) throw new Error("expected one subscription");
    sub.callbacks.onReady?.({
      articles: [article("a1")],
      total: 1,
      stale: false,
    } as ReadyPayload);
    await waitFor(() =>
      expect(useNewsStore.getState().feed.map((a) => a.id)).toEqual(["a1"]),
    );
    sub.callbacks.onArticle?.(article("a2"));
    await waitFor(() =>
      expect(useNewsStore.getState().feed.map((a) => a.id)).toEqual([
        "a2",
        "a1",
      ]),
    );
  });
});

describe("LiveNewsPanel — severity floor", () => {
  test("clicking a chip closes the old subscription and opens a new one with severity in the URL", async () => {
    signedInTier1User();
    const { subscribe, subs } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    expect(subs.length).toBe(1);
    expect(subs[0]?.url).toBe("/api/news/v1/list-live");
    const btn = document.querySelector(
      'button[data-severity-floor="critical"]',
    ) as HTMLButtonElement;
    fireEvent.click(btn);
    await waitFor(() => expect(subs.length).toBe(2));
    expect(subs[0]?.closed).toBe(true);
    expect(subs[1]?.url).toBe("/api/news/v1/list-live?severity=critical");
  });

  test("clicking the same severity twice toggles it back off (no severity in URL)", async () => {
    signedInTier1User();
    const { subscribe, subs } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    const btn = document.querySelector(
      'button[data-severity-floor="warn"]',
    ) as HTMLButtonElement;
    fireEvent.click(btn);
    fireEvent.click(btn);
    await waitFor(() => expect(subs.length).toBe(3));
    expect(subs[2]?.url).toBe("/api/news/v1/list-live");
  });

  test("'All' button is aria-pressed by default", () => {
    signedInTier1User();
    const { subscribe } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    const all = document.querySelector(
      'button[data-severity-floor="all"]',
    ) as HTMLButtonElement;
    expect(all.getAttribute("aria-pressed")).toBe("true");
  });
});

describe("LiveNewsPanel — locked tier path", () => {
  test("anonymous user (tier 0) sees the locked state without opening a subscription", () => {
    // No signIn → tier = 0.
    const { subscribe, subs } = fakeSubscribeFactory();
    render(<LiveNewsPanel subscribe={subscribe} now={NOW_MS} />);
    expect(subs.length).toBe(0);
    expect(document.querySelector('[data-state="locked"]')).not.toBeNull();
  });

  test("REQUIRED_TIER is 1 (Free / signed-in)", () => {
    expect(REQUIRED_TIER).toBe(1);
  });
});
