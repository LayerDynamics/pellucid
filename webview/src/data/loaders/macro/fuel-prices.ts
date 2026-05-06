/** Fuel prices loader. */

export interface FuelPriceRow {
  region: string;
  product: string;
  usdPerGallon: number;
  weekOverWeekChangePct: number;
}

export interface FuelPricesResponse {
  rows: FuelPriceRow[];
  total: number;
  assembledAtMs: number;
  stale: boolean;
}

export type FuelPricesOutcome =
  | { kind: "ready"; response: FuelPricesResponse }
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

export async function loadFuelPrices(opts: LoadOptions = {}): Promise<FuelPricesOutcome> {
  const url = `${opts.baseUrl ?? ""}/api/economic/v1/fuel-prices`;
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
    try { return { kind: "ready", response: (await resp.json()) as FuelPricesResponse }; }
    catch (e) { return { kind: "error", code: "unknown", message: `parse: ${e instanceof Error ? e.message : String(e)}`, httpStatus, retryAfterSecs }; }
  }
  let envelope: { error?: { code?: string; message?: string } } = {};
  try { envelope = (await resp.json()) as typeof envelope; } catch { /* */ }
  const rawCode = envelope?.error?.code ?? null;
  const code: ErrorCode = rawCode && ERROR_CODES.has(rawCode) ? (rawCode as ErrorCode) : "unknown";
  return { kind: "error", code, message: envelope?.error?.message ?? `HTTP ${httpStatus}`, httpStatus, retryAfterSecs: Number.isFinite(retryAfterSecs) ? retryAfterSecs : null };
}
