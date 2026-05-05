/**
 * Backtest-stock loader — wraps `/api/market/v1/backtest-stock`.
 *
 * Mirrors the analyze-stock + list-articles loader shape: the
 * loader returns a discriminated outcome, never throws, encodes
 * optional filters into the query string, normalises every error
 * envelope into a stable `ErrorCode`.
 *
 * Wire shape mirrors
 * `crates/pellucid-handlers/src/market/v1/backtest_stock.rs`.
 */

/** Strategy id — mirrors the Rust `Strategy` enum's serde
 *  `kebab-case` rename. */
export type Strategy = "equal-weight" | "momentum" | "mean-reversion";

/** One symbol's pick within a strategy. */
export interface Pick {
  symbol: string;
  weight: number;
  percentChange: number;
  contribution: number;
}

export interface StrategyMetrics {
  totalReturnPct: number;
  winRatePct: number;
  maxDrawdownPct: number;
}

export interface StrategyResult {
  strategy: Strategy;
  picks: Pick[];
  metrics: StrategyMetrics;
}

export interface BacktestStockResponse {
  strategies: StrategyResult[];
  universe: string[];
  stale: boolean;
  assembledAtMs: number;
}

export interface BacktestStockQuery {
  /** Cap on universe size. Server clamps to MAX_UNIVERSE=50. */
  limitUniverse?: number;
  /** Comma-separated allow-list (case-insensitive). */
  symbols?: string;
}

export type BacktestStockOutcome =
  | { kind: "ready"; response: BacktestStockResponse }
  | {
      kind: "error";
      code: ErrorCode;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

export type ErrorCode =
  | "invalid_request"
  | "cache_failure"
  | "cache_shape"
  | "empty_universe"
  | "bootstrap_upstream_empty"
  | "entitlement_forbidden"
  | "clerk_unauthorized"
  | "network"
  | "unknown";

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
  "empty_universe",
  "bootstrap_upstream_empty",
  "entitlement_forbidden",
  "clerk_unauthorized",
]);

export async function loadBacktestStock(
  q: BacktestStockQuery = {},
  opts: LoadOptions = {},
): Promise<BacktestStockOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  if (typeof q.limitUniverse === "number" && Number.isFinite(q.limitUniverse)) {
    params.set("limitUniverse", String(Math.max(1, Math.floor(q.limitUniverse))));
  }
  if (q.symbols && q.symbols.length > 0) params.set("symbols", q.symbols);
  const qs = params.toString();
  const url = `${baseUrl}/api/market/v1/backtest-stock${qs ? `?${qs}` : ""}`;
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
      const body = (await resp.json()) as BacktestStockResponse;
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
    /* fall through */
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
