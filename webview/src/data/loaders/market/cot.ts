/**
 * COT positioning loader — wraps `/api/market/v1/cot`.
 *
 * Mirrors the wire shape of
 * `crates/pellucid-handlers/src/market/v1/cot.rs`. Same
 * discriminated outcome shape every market loader uses; the
 * panel branches exhaustively on `outcome.kind`.
 */

/** Sort modes — kebab-case wire strings. */
export type SortMode =
  | "managed-money-net-desc"
  | "managed-money-net-asc"
  | "open-interest-desc"
  | "name-asc";

/** One row in the wire response. */
export interface CotRow {
  contractCode: string;
  contractName: string;
  /** Report week (`YYYY-MM-DD`). */
  reportDate: string;
  openInterestAll: number;
  producerLong: number;
  producerShort: number;
  swapLong: number;
  swapShort: number;
  managedMoneyLong: number;
  managedMoneyShort: number;
  /** Pre-computed `managedMoneyLong − managedMoneyShort`. */
  managedMoneyNet: number;
  /** Net managed-money as % of open interest. `0` when
   *  `openInterestAll` is `0`. */
  managedMoneyNetPctOi: number;
}

/** Wire-format response. */
export interface CotResponse {
  rows: CotRow[];
  /** Total rows in the cache slot before the limit clamp. */
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

/** Optional query knobs. */
export interface CotQuery {
  /** Cap on rows. Server clamps to MAX_LIMIT=100. */
  limit?: number;
  /** Sort mode. Server defaults to `managed-money-net-desc`
   *  when absent; unknown values surface a typed 400. */
  sort?: SortMode;
  /** Comma-separated allow-list of CFTC contract codes. */
  contracts?: string;
}

/** Discriminated outcome — never throws. */
export type CotOutcome =
  | { kind: "ready"; response: CotResponse }
  | {
      kind: "error";
      code: ErrorCode;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

/** Stable error codes — same shape as the other anonymous-tier
 *  market loaders. */
export type ErrorCode =
  | "invalid_request"
  | "cache_failure"
  | "cache_shape"
  | "bootstrap_upstream_empty"
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
]);

/**
 * Fetch the COT snapshot. Never throws.
 */
export async function loadCot(
  q: CotQuery = {},
  opts: LoadOptions = {},
): Promise<CotOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  if (typeof q.limit === "number" && Number.isFinite(q.limit)) {
    params.set("limit", String(Math.max(1, Math.floor(q.limit))));
  }
  if (q.sort) params.set("sort", q.sort);
  if (q.contracts && q.contracts.length > 0) params.set("contracts", q.contracts);
  const qs = params.toString();
  const url = `${baseUrl}/api/market/v1/cot${qs ? `?${qs}` : ""}`;
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
      const body = (await resp.json()) as CotResponse;
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
