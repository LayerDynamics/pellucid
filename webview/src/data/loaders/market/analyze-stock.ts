/**
 * Stock analysis loader — single source of truth for the
 * `/api/market/v1/analyze-stock` call from the webview.
 *
 * Mirrors the discriminated-outcome shape of every other v1
 * loader. Wire types match `AnalyzeStockResponse` in
 * `crates/pellucid-handlers/src/market/v1/analyze_stock.rs`
 * 1:1 (camelCase on both sides).
 *
 * The endpoint is tier-2-gated at the gateway layer; this
 * loader does NOT pre-check (the panel surfaces an
 * `entitlement_forbidden` error envelope when the gate denies).
 */

/** Trend classification — direction band of the % change. */
export type Trend = "up" | "down" | "flat";

/** Magnitude band of the absolute % change. */
export type Magnitude = "small" | "medium" | "large";

/** Derived metrics — pure projection of the cached quote. */
export interface Metrics {
  dollarChange: number;
  percentChange: number;
  trend: Trend;
  magnitude: Magnitude;
  /** Position of `price` inside the basket's `[min, max]`
   *  range, normalised to `[0.0, 1.0]`. `0.5` for single-row
   *  baskets. */
  rangePosition: number;
}

/** Wire-format response. Mirrors `AnalyzeStockResponse`. */
export interface AnalyzeStockResponse {
  /** Echo of the requested symbol, canonicalised (uppercased). */
  symbol: string;
  price: number;
  previousClose: number;
  currency: string;
  exchange: string;
  /** Wall-clock ms when the upstream stamped the quote. */
  regularMarketTimeMs: number;
  metrics: Metrics;
  /** Wall-clock ms when the seeder assembled the snapshot. */
  assembledAtMs: number;
  /** True when the response was synthesised from a stale row. */
  stale: boolean;
}

/** Required + optional query knobs. */
export interface AnalyzeStockQuery {
  /** Ticker symbol. Required. */
  symbol: string;
}

/** Discriminated outcome — never throws. */
export type AnalyzeStockOutcome =
  | { kind: "ready"; response: AnalyzeStockResponse }
  | {
      kind: "error";
      code: ErrorCode;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

/** Stable error codes — superset of the other v1 loaders to
 *  cover the analyze-stock-specific `symbol_not_found` 404 +
 *  the gateway's tier-gated `entitlement_forbidden` 403. */
export type ErrorCode =
  | "invalid_request"
  | "symbol_not_found"
  | "cache_failure"
  | "cache_shape"
  | "bootstrap_upstream_empty"
  | "entitlement_forbidden"
  | "entitlement_upstream_down"
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
  "symbol_not_found",
  "cache_failure",
  "cache_shape",
  "bootstrap_upstream_empty",
  "entitlement_forbidden",
  "entitlement_upstream_down",
  "clerk_unauthorized",
]);

/**
 * Fetch the per-symbol stock analysis. Never throws.
 *
 * Returns an `invalid_request` error outcome (without a network
 * call) when `query.symbol` is empty / whitespace.
 */
export async function loadAnalyzeStock(
  query: AnalyzeStockQuery,
  opts: LoadOptions = {},
): Promise<AnalyzeStockOutcome> {
  if (typeof query.symbol !== "string" || query.symbol.trim().length === 0) {
    return {
      kind: "error",
      code: "invalid_request",
      message: "symbol query parameter is required",
      httpStatus: 400,
      retryAfterSecs: null,
    };
  }
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  params.set("symbol", query.symbol.trim().toUpperCase());
  const url = `${baseUrl}/api/market/v1/analyze-stock?${params.toString()}`;
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
      const body = (await resp.json()) as AnalyzeStockResponse;
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
