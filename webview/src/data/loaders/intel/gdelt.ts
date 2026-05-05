/**
 * GDELT intelligence loader — single source of truth for the
 * `/api/intelligence/v1/gdelt-feed` call from the webview.
 *
 * Mirrors the discriminated-outcome shape of
 * `loaders/news/list.ts` so the panel branches exhaustively on
 * `outcome.kind`. Wire types match `GdeltFeedResponse` in
 * `crates/pellucid-handlers/src/intelligence/v1/gdelt_feed.rs`
 * 1:1 (camelCase on both sides).
 */

/** One row in the wire response. Mirrors `GdeltArticle` in the
 *  Rust handler. */
export interface GdeltArticle {
  url: string;
  title: string;
  /** GDELT `YYYYMMDDTHHMMSSZ` timestamp. */
  seenDate: string;
  /** Social-share image URL — empty string when GDELT did not
   *  provide one (the wire never elides this field). */
  socialImage: string;
  domain: string;
  language: string;
  sourceCountry: string;
}

/** Wire-format response. Mirrors `GdeltFeedResponse`. */
export interface GdeltFeedResponse {
  rows: GdeltArticle[];
  /** Echo of the seeder's query string. */
  query: string;
  /** Echo of the seeder's lookback window. */
  timespan: string;
  /** Wall-clock ms when the seeder assembled the snapshot. */
  assembledAtMs: number;
  /** Total rows in the cache slot before the limit clamp. */
  total: number;
  /** True when the response was synthesised from a stale cache row. */
  stale: boolean;
}

/** Optional query knobs. */
export interface GdeltFeedQuery {
  /** Cap on rows returned. Server clamps to [1, 200]. */
  limit?: number;
  /** Filter by GDELT-reported source country (case-insensitive,
   *  trim-tolerant on the server). */
  country?: string;
}

/** Discriminated outcome — never throws, always returns one. */
export type GdeltFeedOutcome =
  | { kind: "ready"; response: GdeltFeedResponse }
  | {
      kind: "error";
      code: ErrorCode;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

/** Stable error codes — same set as the news loaders so the
 *  panel can share branching code across the v1 surface. */
export type ErrorCode =
  | "invalid_request"
  | "cache_failure"
  | "cache_shape"
  | "bootstrap_upstream_empty"
  | "entitlement_forbidden"
  | "clerk_unauthorized"
  | "network"
  | "unknown";

/** Loader options — DI hooks for tests + base URL override. */
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
 * Fetch the GDELT intel feed. Never throws.
 */
export async function loadGdeltFeed(
  q: GdeltFeedQuery = {},
  opts: LoadOptions = {},
): Promise<GdeltFeedOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  if (typeof q.limit === "number" && Number.isFinite(q.limit)) {
    params.set("limit", String(Math.max(1, Math.floor(q.limit))));
  }
  if (q.country && q.country.trim().length > 0) {
    params.set("country", q.country.trim());
  }
  const qs = params.toString();
  const url = `${baseUrl}/api/intelligence/v1/gdelt-feed${qs ? `?${qs}` : ""}`;
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
      const body = (await resp.json()) as GdeltFeedResponse;
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

/**
 * Parse a GDELT `YYYYMMDDTHHMMSSZ` seen-date string into wall-
 * clock ms. Returns `null` when the input doesn't match the
 * expected shape.
 *
 * Exported so the panel can render relative timestamps via the
 * shared `NewsCard` formatter without re-implementing the parser.
 */
export function parseGdeltSeenDate(seenDate: string): number | null {
  // Format: 20260504T120000Z (15 chars: 8 date + T + 6 time + Z).
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/.exec(seenDate);
  if (!m) return null;
  const [, y, mo, d, h, mi, s] = m;
  const ms = Date.UTC(
    Number(y),
    Number(mo) - 1,
    Number(d),
    Number(h),
    Number(mi),
    Number(s),
  );
  return Number.isFinite(ms) ? ms : null;
}
