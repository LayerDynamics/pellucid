/**
 * Country brief loader — single source of truth for the
 * `/api/intelligence/v1/country-brief` call from the webview.
 *
 * Mirrors the discriminated-outcome shape of the deep-dive
 * loader (and intentionally shares the same error code surface
 * so the brief + deep-dive panels can share branching code).
 */

/** Top-1 actor scoped to the requested country. */
export interface TopActor {
  name: string;
  events: number;
}

/** Top-1 (freshest) GDELT article. */
export interface TopArticle {
  url: string;
  title: string;
  domain: string;
  /** GDELT `YYYYMMDDTHHMMSSZ` timestamp. */
  seenDate: string;
}

/** Aggregated totals — same shape as the deep-dive endpoint. */
export interface CountryTotals {
  events: number;
  incidents: number;
  messages: number;
}

/** Wire-format response. Mirrors `CountryBriefResponse`. */
export interface CountryBriefResponse {
  /** Echo of the requested country (canonicalised). */
  country: string;
  /** Coarse region label. */
  region: string;
  /** One-sentence summary the panel renders verbatim. */
  summary: string;
  /** Highest-event actor or `null`. */
  topActor: TopActor | null;
  /** Freshest article or `null`. */
  topArticle: TopArticle | null;
  totals: CountryTotals;
  assembledAtMs: number;
  stale: boolean;
}

/** Required query knob. */
export interface CountryBriefQuery {
  /** Country name. Required. */
  country: string;
}

/** Discriminated outcome — never throws. */
export type CountryBriefOutcome =
  | { kind: "ready"; response: CountryBriefResponse }
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
 * Fetch the country brief. Never throws.
 *
 * Returns an `invalid_request` error outcome (without a network
 * call) when `query.country` is empty / whitespace.
 */
export async function loadCountryBrief(
  query: CountryBriefQuery,
  opts: LoadOptions = {},
): Promise<CountryBriefOutcome> {
  if (typeof query.country !== "string" || query.country.trim().length === 0) {
    return {
      kind: "error",
      code: "invalid_request",
      message: "country query parameter is required",
      httpStatus: 400,
      retryAfterSecs: null,
    };
  }
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  params.set("country", query.country.trim());
  const url = `${baseUrl}/api/intelligence/v1/country-brief?${params.toString()}`;
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
      const body = (await resp.json()) as CountryBriefResponse;
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
