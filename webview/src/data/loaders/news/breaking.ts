/**
 * Breaking-news loader — single source of truth for the
 * `/api/news/v1/get-breaking` call from the webview.
 *
 * Mirrors the discriminated-outcome shape of `list.ts` so the
 * panel can branch exhaustively on `outcome.kind`. The wire
 * response is identical to `list-articles` (same `articles[]` /
 * `total` / `stale` envelope) — only the server-side defaults
 * differ (smaller cap, severity-floor of `high` by default,
 * extra `?sinceMs=` filter for polling).
 */

import type { NewsArticle } from "./list";
import type { SignalSeverity } from "../../../panels/news/SignalSeverityBadge";

/** Wire-format response. Mirrors `GetBreakingResponse` in
 *  `crates/pellucid-handlers/src/news/v1/get_breaking.rs`. */
export interface GetBreakingResponse {
  articles: NewsArticle[];
  /** Count of items that passed the severity + `since_ms`
   *  filters before the limit clamp — drives the "+N more" hint. */
  total: number;
  /** True when the underlying cache row was stale. */
  stale: boolean;
}

/** Optional query knobs. */
export interface GetBreakingQuery {
  /** Cap on the number of articles returned. Server defaults to 5
   *  and clamps to [1, MAX_LIMIT=20]. */
  limit?: number;
  /** Severity floor. Server defaults to `high` (i.e. `info` and
   *  `warn` are filtered out by default). */
  severity?: SignalSeverity;
  /** Wall-clock ms cutoff. When set, articles whose
   *  `publishedAtMs` is `<=` this value are filtered out. The
   *  banner uses the freshest article it has already rendered as
   *  the cutoff so a poll only delivers truly-new items. */
  sinceMs?: number;
}

/** Discriminated outcome — never throws, always returns one of
 *  these. Matches `ListArticlesOutcome` in `list.ts`. */
export type GetBreakingOutcome =
  | { kind: "ready"; response: GetBreakingResponse }
  | {
      kind: "error";
      code: ErrorCode;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

/** Stable error codes the panel branches on. Same code surface
 *  as `list.ts` so the banner can share branching code with the
 *  static news panel. */
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
 * Fetch the breaking-news article list. Never throws; on any
 * failure (network, 4xx, 5xx, malformed body) returns
 * `{ kind: "error", ... }`.
 */
export async function loadBreakingNews(
  q: GetBreakingQuery = {},
  opts: LoadOptions = {},
): Promise<GetBreakingOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  if (typeof q.limit === "number" && Number.isFinite(q.limit)) {
    params.set("limit", String(Math.max(1, Math.floor(q.limit))));
  }
  if (q.severity) params.set("severity", q.severity);
  if (typeof q.sinceMs === "number" && Number.isFinite(q.sinceMs)) {
    params.set("sinceMs", String(Math.floor(q.sinceMs)));
  }
  const qs = params.toString();
  const url = `${baseUrl}/api/news/v1/get-breaking${qs ? `?${qs}` : ""}`;
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
      const body = (await resp.json()) as GetBreakingResponse;
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
