/**
 * Yield curve loader — wraps `/api/market/v1/yield-curve`.
 */

export interface YieldPoint {
  seriesCode: string;
  maturityLabel: string;
  maturityMonths: number;
  yieldPct: number;
  observationDate: string;
}

export interface YieldSpreads {
  tenMinusTwo?: number;
  tenMinusThreeMonth?: number;
  thirtyMinusFive?: number;
}

export interface YieldCurveResponse {
  points: YieldPoint[];
  spreads: YieldSpreads;
  inverted: boolean;
  assembledAtMs: number;
  stale: boolean;
}

export type YieldCurveOutcome =
  | { kind: "ready"; response: YieldCurveResponse }
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
  | "bootstrap_upstream_empty"
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
  "bootstrap_upstream_empty",
]);

export async function loadYieldCurve(
  opts: LoadOptions = {},
): Promise<YieldCurveOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const url = `${baseUrl}/api/market/v1/yield-curve`;
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
      const body = (await resp.json()) as YieldCurveResponse;
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
