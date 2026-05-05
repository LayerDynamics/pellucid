/**
 * Breadth loader — wraps `/api/market/v1/breadth`.
 *
 * Mirrors the wire shape declared by
 * `crates/pellucid-handlers/src/market/v1/breadth.rs`. Same
 * discriminated outcome shape every market loader uses.
 */

export interface BreadthRow {
  symbol: string;
  percentChange: number;
}

export interface BreadthResponse {
  advancers: number;
  decliners: number;
  unchanged: number;
  advanceDeclineLine: number;
  newHighs: number;
  newLows: number;
  topAdvancers: BreadthRow[];
  topDecliners: BreadthRow[];
  universe: number;
  stale: boolean;
  assembledAtMs: number;
}

export interface BreadthQuery {
  /** Cap on `topAdvancers` + `topDecliners`. Server clamps to
   *  MAX_TOP_N=25. */
  topN?: number;
}

export type BreadthOutcome =
  | { kind: "ready"; response: BreadthResponse }
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

export async function loadBreadth(
  q: BreadthQuery = {},
  opts: LoadOptions = {},
): Promise<BreadthOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  if (typeof q.topN === "number" && Number.isFinite(q.topN)) {
    params.set("topN", String(Math.max(1, Math.floor(q.topN))));
  }
  const qs = params.toString();
  const url = `${baseUrl}/api/market/v1/breadth${qs ? `?${qs}` : ""}`;
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
      const body = (await resp.json()) as BreadthResponse;
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
