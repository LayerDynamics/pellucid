/** Consumer prices list loader — wraps `/api/consumer-prices/v1/list`. */

export interface CpiComponent {
  label: string;
  yoyPct: number;
  weight?: number;
}

export interface CpiRegionRow {
  region: string;
  period: string;
  headlineValue: number;
  yoyPct: number;
  momPct: number;
  components?: CpiComponent[];
}

export interface CpiListResponse {
  rows: CpiRegionRow[];
  assembledAtMs: number;
  stale: boolean;
}

export type CpiListOutcome =
  | { kind: "ready"; response: CpiListResponse }
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

export async function loadCpiList(opts: LoadOptions = {}): Promise<CpiListOutcome> {
  const url = `${opts.baseUrl ?? ""}/api/consumer-prices/v1/list`;
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
    try { return { kind: "ready", response: (await resp.json()) as CpiListResponse }; }
    catch (e) { return { kind: "error", code: "unknown", message: `parse: ${e instanceof Error ? e.message : String(e)}`, httpStatus, retryAfterSecs }; }
  }
  let envelope: { error?: { code?: string; message?: string } } = {};
  try { envelope = (await resp.json()) as typeof envelope; } catch { /* */ }
  const rawCode = envelope?.error?.code ?? null;
  const code: ErrorCode = rawCode && ERROR_CODES.has(rawCode) ? (rawCode as ErrorCode) : "unknown";
  return { kind: "error", code, message: envelope?.error?.message ?? `HTTP ${httpStatus}`, httpStatus, retryAfterSecs: Number.isFinite(retryAfterSecs) ? retryAfterSecs : null };
}
