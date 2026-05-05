/**
 * Market quotes loader — single source of truth for the
 * `/api/market/v1/list-market-quotes` call from the webview.
 *
 * Mirrors the discriminated-outcome shape of every other v1
 * loader. Wire types match `ListMarketQuotesResponse` in
 * `crates/pellucid-handlers/src/market/v1/list_market_quotes.rs`
 * 1:1 (camelCase on both sides).
 */

/** One quote row in the wire response. Mirrors `MarketQuote`. */
export interface MarketQuote {
  symbol: string;
  price: number;
  previousClose: number;
  /** Pre-computed % change vs `previousClose`. */
  percentChange: number;
  currency: string;
  exchange: string;
  /** Wall-clock ms when the upstream stamped this row. */
  regularMarketTimeMs: number;
}

/** Wire-format response. Mirrors `ListMarketQuotesResponse`. */
export interface ListMarketQuotesResponse {
  rows: MarketQuote[];
  /** Wall-clock ms when the seeder assembled the snapshot. */
  assembledAtMs: number;
  /** Total quotes in the cache slot before the limit clamp. */
  total: number;
  /** True when the response was synthesised from a stale row. */
  stale: boolean;
}

/** Optional query knobs. */
export interface ListMarketQuotesQuery {
  /** Cap on rows. Server clamps to [1, 100]. */
  limit?: number;
  /** Comma-separated allow-list of ticker symbols (case-
   *  insensitive on the server). */
  symbols?: string[];
}

/** Discriminated outcome — never throws. */
export type ListMarketQuotesOutcome =
  | { kind: "ready"; response: ListMarketQuotesResponse }
  | {
      kind: "error";
      code: ErrorCode;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

/** Stable error codes — same set as every other v1 loader. */
export type ErrorCode =
  | "invalid_request"
  | "cache_failure"
  | "cache_shape"
  | "bootstrap_upstream_empty"
  | "entitlement_forbidden"
  | "clerk_unauthorized"
  | "network"
  | "unknown";

/** Loader options. */
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
 * Fetch the market quotes snapshot. Never throws.
 */
export async function loadMarketQuotes(
  q: ListMarketQuotesQuery = {},
  opts: LoadOptions = {},
): Promise<ListMarketQuotesOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  if (typeof q.limit === "number" && Number.isFinite(q.limit)) {
    params.set("limit", String(Math.max(1, Math.floor(q.limit))));
  }
  if (q.symbols && q.symbols.length > 0) {
    const csv = q.symbols
      .map((s) => s.trim())
      .filter((s) => s.length > 0)
      .join(",");
    if (csv.length > 0) params.set("symbols", csv);
  }
  const qs = params.toString();
  const url = `${baseUrl}/api/market/v1/list-market-quotes${qs ? `?${qs}` : ""}`;
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
      const body = (await resp.json()) as ListMarketQuotesResponse;
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
