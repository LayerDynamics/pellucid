import { afterEach, describe, expect, test } from "bun:test";

import {
  subscribe,
  type EventSourceFactory,
  type LiveError,
  type ReadyPayload,
  type SubscribableEventSource,
} from "./live";
import type { NewsArticle } from "./list";

/** Tiny EventSource fake — registers listeners + lets the test
 *  fire events into them. Mirrors the subset of the EventSource
 *  interface the loader actually exercises. */
class FakeEventSource implements SubscribableEventSource {
  readyState = 0;
  url: string;
  closed = false;
  closeCount = 0;
  private listeners = new Map<string, Set<(e: Event | MessageEvent) => void>>();

  constructor(url: string) {
    this.url = url;
  }

  addEventListener(
    type: string,
    listener: (e: Event | MessageEvent) => void,
  ): void {
    let set = this.listeners.get(type);
    if (!set) {
      set = new Set();
      this.listeners.set(type, set);
    }
    set.add(listener);
  }

  removeEventListener(
    type: string,
    listener: (e: Event | MessageEvent) => void,
  ): void {
    this.listeners.get(type)?.delete(listener);
  }

  close(): void {
    this.closed = true;
    this.closeCount++;
  }

  /** Test helper — fire a typed message event into the listener
   *  registered for `type` (mirrors what the SSE parser does). */
  emit(type: string, data: string): void {
    const set = this.listeners.get(type);
    if (!set) return;
    const ev = new MessageEvent(type, { data });
    for (const listener of set) listener(ev);
  }

  /** Test helper — fire a generic Event (no `data`) into the
   *  `error` channel, simulating a connection drop. */
  emitConnectionError(): void {
    const set = this.listeners.get("error");
    if (!set) return;
    const ev = new Event("error");
    for (const listener of set) listener(ev);
  }

  listenerCount(type: string): number {
    return this.listeners.get(type)?.size ?? 0;
  }
}

function makeFactory(): {
  factory: EventSourceFactory;
  instances: FakeEventSource[];
} {
  const instances: FakeEventSource[] = [];
  const factory: EventSourceFactory = (url) => {
    const es = new FakeEventSource(url);
    instances.push(es);
    return es;
  };
  return { factory, instances };
}

afterEach(() => {
  /* nothing global to reset */
});

describe("subscribe", () => {
  test("opens an EventSource at the documented path with no query when filters absent", () => {
    const { factory, instances } = makeFactory();
    const sub = subscribe({}, {}, { eventSourceImpl: factory });
    expect(instances.length).toBe(1);
    const first = instances[0];
    if (!first) throw new Error("expected one instance");
    expect(first.url).toBe("/api/news/v1/list-live");
    expect(sub.url).toBe(first.url);
    sub.close();
  });

  test("encodes ?severity + ?limit into the URL", () => {
    const { factory, instances } = makeFactory();
    subscribe(
      {},
      { severity: "critical", limit: 100 },
      { eventSourceImpl: factory },
    );
    const first = instances[0];
    if (!first) throw new Error("expected one instance");
    expect(first.url).toContain("severity=critical");
    expect(first.url).toContain("limit=100");
  });

  test("clamps non-finite or fractional limit to floor (>=1)", () => {
    const { factory, instances } = makeFactory();
    subscribe({}, { limit: 0 }, { eventSourceImpl: factory });
    const first = instances[0];
    if (!first) throw new Error("expected one instance");
    expect(first.url).toContain("limit=1");
  });

  test("forwards bearer token via ?_t=", () => {
    const { factory, instances } = makeFactory();
    subscribe(
      {},
      {},
      { eventSourceImpl: factory, bearerToken: "tok-xyz" },
    );
    const first = instances[0];
    if (!first) throw new Error("expected one instance");
    expect(first.url).toContain("_t=tok-xyz");
  });

  test("baseUrl prefixes the request URL", () => {
    const { factory, instances } = makeFactory();
    subscribe(
      {},
      {},
      { eventSourceImpl: factory, baseUrl: "https://api.example" },
    );
    const first = instances[0];
    if (!first) throw new Error("expected one instance");
    expect(first.url).toBe("https://api.example/api/news/v1/list-live");
  });

  test("ready event triggers onReady with the parsed snapshot", () => {
    const captured: ReadyPayload[] = [];
    const { factory, instances } = makeFactory();
    subscribe(
      {
        onReady: (snap) => {
          captured.push(snap);
        },
      },
      {},
      { eventSourceImpl: factory },
    );
    const es = instances[0];
    if (!es) throw new Error("expected one instance");
    es.emit(
      "ready",
      JSON.stringify({
        articles: [
          {
            id: "a1",
            title: "Suez convoy",
            source: "Reuters",
            publishedAtMs: 1,
          },
        ],
        total: 1,
        stale: false,
      }),
    );
    expect(captured.length).toBe(1);
    const first = captured[0];
    if (!first) throw new Error("expected one ready payload");
    expect(first.articles.length).toBe(1);
    expect(first.articles[0]?.id).toBe("a1");
    expect(first.total).toBe(1);
    expect(first.stale).toBe(false);
  });

  test("article event triggers onArticle with the parsed delta", () => {
    const captured: NewsArticle[] = [];
    const { factory, instances } = makeFactory();
    subscribe(
      {
        onArticle: (a) => {
          captured.push(a);
        },
      },
      {},
      { eventSourceImpl: factory },
    );
    const es = instances[0];
    if (!es) throw new Error("expected one instance");
    es.emit(
      "article",
      JSON.stringify({
        id: "a-new",
        title: "Wildfire alert",
        source: "AP",
        publishedAtMs: 2,
        severity: "critical",
      }),
    );
    expect(captured.length).toBe(1);
    expect(captured[0]?.id).toBe("a-new");
    expect(captured[0]?.severity).toBe("critical");
  });

  test("outage event triggers onOutage with the reason string", () => {
    const reasons: string[] = [];
    const { factory, instances } = makeFactory();
    subscribe(
      { onOutage: (r) => reasons.push(r) },
      {},
      { eventSourceImpl: factory },
    );
    const es = instances[0];
    if (!es) throw new Error("expected one instance");
    es.emit("outage", JSON.stringify({ reason: "bootstrap_upstream_empty" }));
    expect(reasons).toEqual(["bootstrap_upstream_empty"]);
  });

  test("outage with malformed JSON falls back to default reason", () => {
    const reasons: string[] = [];
    const { factory, instances } = makeFactory();
    subscribe(
      { onOutage: (r) => reasons.push(r) },
      {},
      { eventSourceImpl: factory },
    );
    const es = instances[0];
    if (!es) throw new Error("expected one instance");
    es.emit("outage", "not json");
    expect(reasons).toEqual(["bootstrap_upstream_empty"]);
  });

  test("error event with payload triggers onError with the parsed envelope", () => {
    const errs: LiveError[] = [];
    const { factory, instances } = makeFactory();
    subscribe(
      { onError: (e) => errs.push(e) },
      {},
      { eventSourceImpl: factory },
    );
    const es = instances[0];
    if (!es) throw new Error("expected one instance");
    es.emit(
      "error",
      JSON.stringify({ code: "cache_failure", message: "db locked" }),
    );
    expect(errs.length).toBe(1);
    expect(errs[0]?.code).toBe("cache_failure");
  });

  test("ready event with malformed body triggers onError parse_error", () => {
    const errs: LiveError[] = [];
    const { factory, instances } = makeFactory();
    subscribe(
      { onError: (e) => errs.push(e) },
      {},
      { eventSourceImpl: factory },
    );
    const es = instances[0];
    if (!es) throw new Error("expected one instance");
    es.emit("ready", "not-json-at-all");
    expect(errs.length).toBe(1);
    expect(errs[0]?.code).toBe("parse_error");
    expect(errs[0]?.message).toContain("ready:");
  });

  test("connection-error (no data) routes to onConnectionError", () => {
    const conn: Event[] = [];
    const handlerErrs: LiveError[] = [];
    const { factory, instances } = makeFactory();
    subscribe(
      {
        onConnectionError: (e) => conn.push(e),
        onError: (e) => handlerErrs.push(e),
      },
      {},
      { eventSourceImpl: factory },
    );
    const es = instances[0];
    if (!es) throw new Error("expected one instance");
    es.emitConnectionError();
    expect(conn.length).toBe(1);
    expect(handlerErrs.length).toBe(0);
  });

  test("close() drops all listeners and the EventSource", () => {
    const { factory, instances } = makeFactory();
    const sub = subscribe({}, {}, { eventSourceImpl: factory });
    const es = instances[0];
    if (!es) throw new Error("expected one instance");
    expect(es.listenerCount("ready")).toBe(1);
    expect(es.listenerCount("article")).toBe(1);
    expect(es.listenerCount("outage")).toBe(1);
    expect(es.listenerCount("error")).toBe(1);
    sub.close();
    expect(es.closed).toBe(true);
    expect(es.listenerCount("ready")).toBe(0);
    expect(es.listenerCount("article")).toBe(0);
    expect(es.listenerCount("outage")).toBe(0);
    expect(es.listenerCount("error")).toBe(0);
  });

  test("close() is idempotent — calling twice closes the underlying ES once", () => {
    const { factory, instances } = makeFactory();
    const sub = subscribe({}, {}, { eventSourceImpl: factory });
    const es = instances[0];
    if (!es) throw new Error("expected one instance");
    sub.close();
    sub.close();
    sub.close();
    expect(es.closeCount).toBe(1);
  });

  test("events received after close() are dropped", () => {
    const articles: NewsArticle[] = [];
    const { factory, instances } = makeFactory();
    const sub = subscribe(
      { onArticle: (a) => articles.push(a) },
      {},
      { eventSourceImpl: factory },
    );
    const es = instances[0];
    if (!es) throw new Error("expected one instance");
    es.emit(
      "article",
      JSON.stringify({
        id: "a1",
        title: "x",
        source: "x",
        publishedAtMs: 0,
      }),
    );
    expect(articles.length).toBe(1);
    sub.close();
    // Even if a stray event leaks through after close (the
    // listener may still be called by an in-flight emit), the
    // loader must NOT forward it.
    es.emit(
      "article",
      JSON.stringify({
        id: "a2",
        title: "y",
        source: "y",
        publishedAtMs: 0,
      }),
    );
    expect(articles.length).toBe(1);
  });
});
