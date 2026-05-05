/**
 * News list loader — single source of truth for the
 * `/api/news/v1/list-articles` call from the webview.
 *
 * Mirrors `webview/src/data/loaders/aviation.ts` in shape: the
 * loader returns a discriminated union, never throws on upstream
 * failure — the panel branches on `outcome.kind`.
 *
 * The handler at `crates/pellucid-handlers/src/news/v1/list_articles.rs`
 * defines the wire shape; the types here mirror it 1:1 so the
 * panel can render the response without reshaping.
 */

import type { NewsCardItem } from "../../../panels/news/NewsCard";
import type { SignalSeverity } from "../../../panels/news/SignalSeverityBadge";

/** One article in the response. Mirrors `NewsArticle` in
 *  `crates/pellucid-handlers/src/news/v1/list_articles.rs`. The
 *  shape is also a strict superset of `NewsCardItem` so a panel
 *  can pass an article straight to `<NewsCard item={...} />`. */
export interface NewsArticle extends NewsCardItem {}

/** Response envelope. Mirrors `ListArticlesResponse` in the
 *  handler — the webview reads this verbatim. */
export interface ListArticlesResponse {
  articles: NewsArticle[];
  total: number;
  stale: boolean;
}

/** Optional query knobs. */
export interface ListArticlesQuery {
  /** Cap on the number of articles returned. Server clamps to
   *  [1, MAX_LIMIT=200]. Defaults to 50 server-side. */
  limit?: number;
  /** Optional severity floor. When set, articles below the
   *  requested severity are filtered out before the limit clamp. */
  severity?: SignalSeverity;
}

/** Discriminated outcome — never throws, always returns one of
 *  these. The panel branches exhaustively on `kind`. */
export type ListArticlesOutcome =
  | { kind: "ready"; response: ListArticlesResponse }
  | {
      kind: "error";
      code: ErrorCode;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

/** Stable error codes the panel branches on. Mirrors
 *  `HandlerError::code()` in the handler PLUS the bootstrap
 *  outage code (the news handler emits the same shape). */
export type ErrorCode =
  | "invalid_request"
  | "cache_failure"
  | "cache_shape"
  | "bootstrap_upstream_empty"
  | "entitlement_forbidden"
  | "clerk_unauthorized"
  | "network"
  | "unknown";

/** Loader options — fetch injection + base URL override for
 *  tests, optional bearer token for tier-gated calls. */
export interface LoadOptions {
  baseUrl?: string;
  fetchImpl?: typeof fetch;
  signal?: AbortSignal;
  bearerToken?: string;
}

const ERROR_CODES = new Set<string>([
  "invalid_request",
  "cache_failure",
  "cache_shape",
  "bootstrap_upstream_empty",
  "entitlement_forbidden",
  "clerk_unauthorized",
]);

/**
 * Fetch the news article list. Never throws; on any failure
 * (network, 4xx, 5xx, malformed body) returns
 * `{ kind: "error", ... }`.
 */
export async function loadNewsArticles(
  q: ListArticlesQuery = {},
  opts: LoadOptions = {},
): Promise<ListArticlesOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  if (typeof q.limit === "number" && Number.isFinite(q.limit)) {
    params.set("limit", String(Math.max(1, Math.floor(q.limit))));
  }
  if (q.severity) params.set("severity", q.severity);
  const qs = params.toString();
  const url = `${baseUrl}/api/news/v1/list-articles${qs ? `?${qs}` : ""}`;
  const headers: Record<string, string> = { accept: "application/json" };
  if (opts.bearerToken) headers.authorization = `Bearer ${opts.bearerToken}`;

  let resp: Response;
  try {
    const init: RequestInit = { method: "GET", headers };
    if (opts.signal) init.signal = opts.signal;
    resp = await fetchImpl(url, init);
  } catch (e) {
    return {
      kind: "error",
      code: "network",
      message: e instanceof Error ? e.message : String(e),
      httpStatus: 0,
      retryAfterSecs: null,
    };
  }

  const httpStatus = resp.status;
  const retryAfterRaw = resp.headers.get("retry-after");
  const retryAfterSecs = retryAfterRaw ? Number.parseInt(retryAfterRaw, 10) : null;

  if (httpStatus === 200) {
    try {
      const body = (await resp.json()) as ListArticlesResponse;
      return { kind: "ready", response: body };
    } catch (e) {
      return {
        kind: "error",
        code: "unknown",
        message: `parse: ${e instanceof Error ? e.message : String(e)}`,
        httpStatus,
        retryAfterSecs,
      };
    }
  }

  // Error path — try to read the gateway envelope. If parsing fails
  // too, fall back to a synthesised envelope so the panel still has
  // something to render.
  let envelope: { error?: { code?: string; message?: string } } = {};
  try {
    envelope = (await resp.json()) as typeof envelope;
  } catch {
    /* fall through with empty envelope */
  }
  const rawCode = envelope?.error?.code ?? null;
  const code: ErrorCode =
    rawCode && ERROR_CODES.has(rawCode) ? (rawCode as ErrorCode) : "unknown";
  const message = envelope?.error?.message ?? `HTTP ${httpStatus}`;
  return {
    kind: "error",
    code,
    message,
    httpStatus,
    retryAfterSecs: Number.isFinite(retryAfterSecs) ? retryAfterSecs : null,
  };
}
