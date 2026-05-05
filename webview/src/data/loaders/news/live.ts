/**
 * Live news stream loader — wraps `EventSource` against
 * `/api/news/v1/list-live` and normalises the SSE events into a
 * plain callback API the panel can consume.
 *
 * The handler emits four event types (see
 * `crates/pellucid-handlers/src/news/v1/list_live.rs`):
 *
 *   ready    — initial snapshot { articles, total, stale }
 *   article  — one new NewsArticle (delta)
 *   outage   — { reason: "bootstrap_upstream_empty" }
 *   error    — { code, message }
 *
 * Per the universal test mandate, the loader is dependency-
 * injection-friendly: the [`subscribe`] entry point takes an
 * `eventSourceImpl` factory so tests can pass a hand-rolled
 * fake without standing up a real network connection.
 */

import type { NewsArticle } from "./list";
import type { SignalSeverity } from "../../../panels/news/SignalSeverityBadge";

/** Callbacks the panel registers. Every callback is optional —
 *  a panel that only cares about deltas can ignore `onReady`. */
export interface LiveSubscriberCallbacks {
  /** Initial snapshot — fired exactly once per connection. */
  onReady?: (snapshot: ReadyPayload) => void;
  /** One new article. */
  onArticle?: (article: NewsArticle) => void;
  /** The cache reports M4 outage; pipeline is offline. */
  onOutage?: (reason: string) => void;
  /** A handler-side or transport error. */
  onError?: (err: LiveError) => void;
  /** EventSource itself raised an `error` event (network /
   *  connection drop). The browser's reconnect logic kicks in
   *  automatically; this callback fires on each transition. */
  onConnectionError?: (event: Event) => void;
}

/** Initial snapshot payload — same shape as the `list-articles`
 *  response so the panel can prime its list from the SSE
 *  channel without making a separate REST call. */
export interface ReadyPayload {
  articles: NewsArticle[];
  total: number;
  stale: boolean;
}

/** Error envelope flowing through the SSE `error` event. */
export interface LiveError {
  code: string;
  message: string;
}

/** Optional query knobs — same shape as the REST endpoint so
 *  the panel can hand off the same filter object. */
export interface SubscribeQuery {
  severity?: SignalSeverity;
  /** Initial-snapshot cap; server clamps to MAX_LIMIT=200. */
  limit?: number;
}

/** Subscribe options — DI hooks for tests + base URL override. */
export interface SubscribeOptions {
  baseUrl?: string;
  /** Factory for the underlying transport. Defaults to
   *  `globalThis.EventSource`. Tests inject a fake here. */
  eventSourceImpl?: EventSourceFactory;
  /** Optional bearer token forwarded as `?_t=` (EventSource has
   *  no headers in the spec; falling back to a query parameter
   *  matches the original WorldMonitor pattern). */
  bearerToken?: string;
}

/** Minimal interface the loader needs from an EventSource. The
 *  browser's native `EventSource` satisfies this implicitly;
 *  fakes implement only what's exercised. */
export interface SubscribableEventSource {
  addEventListener: (
    type: string,
    listener: (event: MessageEvent | Event) => void,
  ) => void;
  removeEventListener?: (
    type: string,
    listener: (event: MessageEvent | Event) => void,
  ) => void;
  close: () => void;
  readonly readyState: number;
}

/** Factory signature — a function that returns an
 *  EventSource-compatible object given a URL. */
export type EventSourceFactory = (url: string) => SubscribableEventSource;

/** Handle returned to the caller — call `close()` to drop the
 *  subscription. Idempotent. */
export interface LiveSubscription {
  /** Close the underlying EventSource + drop every listener.
   *  Safe to call multiple times. */
  close: () => void;
  /** The URL the subscription opened against — useful for
   *  debugging + e2e assertions. */
  readonly url: string;
}

/**
 * Subscribe to the live news stream.
 *
 * Returns a [`LiveSubscription`]. The caller is responsible for
 * calling `subscription.close()` when the panel unmounts to
 * release the EventSource and stop the server-side polling loop.
 */
export function subscribe(
  callbacks: LiveSubscriberCallbacks,
  query: SubscribeQuery = {},
  opts: SubscribeOptions = {},
): LiveSubscription {
  const baseUrl = opts.baseUrl ?? "";
  const factory =
    opts.eventSourceImpl ??
    ((url: string) =>
      new (globalThis.EventSource as unknown as new (
        u: string,
      ) => SubscribableEventSource)(url));
  const params = new URLSearchParams();
  if (query.severity) params.set("severity", query.severity);
  if (typeof query.limit === "number" && Number.isFinite(query.limit)) {
    params.set("limit", String(Math.max(1, Math.floor(query.limit))));
  }
  if (opts.bearerToken) params.set("_t", opts.bearerToken);
  const qs = params.toString();
  const url = `${baseUrl}/api/news/v1/list-live${qs ? `?${qs}` : ""}`;

  const es = factory(url);
  let closed = false;

  const onReady = (e: Event) => {
    if (closed) return;
    const m = e as MessageEvent;
    const parsed = parseMessageData<ReadyPayload>(m.data);
    if (parsed.kind === "ok") callbacks.onReady?.(parsed.value);
    else
      callbacks.onError?.({
        code: "parse_error",
        message: `ready: ${parsed.message}`,
      });
  };
  const onArticle = (e: Event) => {
    if (closed) return;
    const m = e as MessageEvent;
    const parsed = parseMessageData<NewsArticle>(m.data);
    if (parsed.kind === "ok") callbacks.onArticle?.(parsed.value);
    else
      callbacks.onError?.({
        code: "parse_error",
        message: `article: ${parsed.message}`,
      });
  };
  const onOutage = (e: Event) => {
    if (closed) return;
    const m = e as MessageEvent;
    const parsed = parseMessageData<{ reason: string }>(m.data);
    const reason =
      parsed.kind === "ok" ? parsed.value.reason : "bootstrap_upstream_empty";
    callbacks.onOutage?.(reason);
  };
  const onErrorEvent = (e: Event) => {
    if (closed) return;
    const m = e as MessageEvent;
    if (typeof m.data === "string" && m.data.length > 0) {
      const parsed = parseMessageData<LiveError>(m.data);
      if (parsed.kind === "ok") {
        callbacks.onError?.(parsed.value);
        return;
      }
    }
    // Native EventSource `error` events have no `.data`; surface
    // them via the connection-error callback so the panel can
    // distinguish a transport drop from a handler-side error.
    callbacks.onConnectionError?.(e);
  };

  es.addEventListener("ready", onReady);
  es.addEventListener("article", onArticle);
  es.addEventListener("outage", onOutage);
  es.addEventListener("error", onErrorEvent);

  return {
    url,
    close: () => {
      if (closed) return;
      closed = true;
      es.removeEventListener?.("ready", onReady);
      es.removeEventListener?.("article", onArticle);
      es.removeEventListener?.("outage", onOutage);
      es.removeEventListener?.("error", onErrorEvent);
      es.close();
    },
  };
}

type ParseResult<T> = { kind: "ok"; value: T } | { kind: "err"; message: string };

function parseMessageData<T>(raw: unknown): ParseResult<T> {
  if (typeof raw !== "string") {
    return { kind: "err", message: "non-string data" };
  }
  try {
    const value = JSON.parse(raw) as T;
    return { kind: "ok", value };
  } catch (e) {
    return {
      kind: "err",
      message: e instanceof Error ? e.message : String(e),
    };
  }
}
