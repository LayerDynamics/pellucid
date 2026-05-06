/** Big Mac loader. */

export interface BigMacRow {
  iso: string;
  country: string;
  localPrice: number;
  currency: string;
  usdPrice: number;
  ppp: number;
  fxRate: number;
  valuationPct: number;
}

export interface BigMacResponse {
  snapshotDate: string;
  rows: BigMacRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export interface BigMacQuery {
  limit?: number;
  iso?: string;
}

export type BigMacOutcome =
  | { kind: "ready"; response: BigMacResponse }
  | { kind: "error"; code: ErrorCode; message: string; httpStatus: number; retryAfterSecs: number | null };

export type ErrorCode =
  | "invalid_request" | "cache_failure" | "cache_shape" | "bootstrap_upstream_empty" | "network" | "unknown";

export interface LoadOptions {
  baseUrl?: string;
  fetchImpl?: typeof fetch;
  signal?: AbortSignal;
  bearerToken?: string;
}

const ERROR_CODES = new Set<string>([
  "invalid_request", "cache_failure", "cache_shape", "bootstrap_upstream_empty",
]);

export async function loadBigMac(q: BigMacQuery = {}, opts: LoadOptions = {}): Promise<BigMacOutcome> {
  const params = new URLSearchParams();
  if (typeof q.limit === "number" && Number.isFinite(q.limit)) params.set("limit", String(Math.max(1, Math.floor(q.limit))));
  if (q.iso && q.iso.length > 0) params.set("iso", q.iso);
  const qs = params.toString();
  const url = `${opts.baseUrl ?? ""}/api/economic/v1/big-mac${qs ? `?${qs}` : ""}`;
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const headers: Record<string, string> = { accept: "application/json" };
  if (opts.bearerToken) headers.authorization = `Bearer ${opts.bearerToken}`;
  let resp: Response;
  try {
    const init: RequestInit = { method: "GET", headers };
    if (opts.signal) init.signal = opts.signal;
    resp = await fetchImpl(url, init);
  } catch (e) {
    return { kind: "error", code: "network", message: e instanceof Error ? e.message : String(e), httpStatus: 0, retryAfterSecs: null };
  }
  const httpStatus = resp.status;
  const ra = resp.headers.get("retry-after");
  const retryAfterSecs = ra ? Number.parseInt(ra, 10) : null;
  if (httpStatus === 200) {
    try { return { kind: "ready", response: (await resp.json()) as BigMacResponse }; }
    catch (e) { return { kind: "error", code: "unknown", message: `parse: ${e instanceof Error ? e.message : String(e)}`, httpStatus, retryAfterSecs }; }
  }
  let envelope: { error?: { code?: string; message?: string } } = {};
  try { envelope = (await resp.json()) as typeof envelope; } catch { /* */ }
  const rawCode = envelope?.error?.code ?? null;
  const code: ErrorCode = rawCode && ERROR_CODES.has(rawCode) ? (rawCode as ErrorCode) : "unknown";
  return { kind: "error", code, message: envelope?.error?.message ?? `HTTP ${httpStatus}`, httpStatus, retryAfterSecs: Number.isFinite(retryAfterSecs) ? retryAfterSecs : null };
}
