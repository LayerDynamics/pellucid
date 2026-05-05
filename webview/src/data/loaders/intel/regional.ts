/**
 * Regional intel loader — single source of truth for the
 * `/api/intelligence/v1/regional` call from the webview.
 *
 * Mirrors the discriminated-outcome shape of the gdelt + telegram
 * loaders. Wire types match `RegionalResponse` in the Rust
 * handler 1:1.
 */

/** Per-country breakdown row inside one region. */
export interface CountryRollup {
  name: string;
  events: number;
  incidents: number;
}

/** One region's rollup. */
export interface RegionRollup {
  region: string;
  totalEvents: number;
  totalIncidents: number;
  countries: CountryRollup[];
  topActors: string[];
}

/** Wire-format response. */
export interface RegionalResponse {
  regions: RegionRollup[];
  /** Maximum of the two upstream `assembled_at_ms` timestamps. */
  assembledAtMs: number;
  /** True when EITHER upstream cache slot was stale. */
  stale: boolean;
}

/** Optional query knobs. */
export interface RegionalQuery {
  /** Filter to a single region label. Case-insensitive,
   *  trim-tolerant on the server. */
  region?: string;
}

/** Discriminated outcome — never throws. */
export type RegionalOutcome =
  | { kind: "ready"; response: RegionalResponse }
  | {
      kind: "error";
      code: ErrorCode;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

/** Stable error codes — same set as the other intel loaders. */
export type ErrorCode =
  | "invalid_request"
  | "cache_failure"
  | "cache_shape"
  | "bootstrap_upstream_empty"
  | "entitlement_forbidden"
  | "clerk_unauthorized"
  | "network"
  | "unknown";

/** Loader options — DI hooks for tests. */
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
 * Fetch the regional rollup. Never throws.
 */
export async function loadRegional(
  q: RegionalQuery = {},
  opts: LoadOptions = {},
): Promise<RegionalOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  if (q.region && q.region.trim().length > 0) {
    params.set("region", q.region.trim());
  }
  const qs = params.toString();
  const url = `${baseUrl}/api/intelligence/v1/regional${qs ? `?${qs}` : ""}`;
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
      const body = (await resp.json()) as RegionalResponse;
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
