/**
 * Country deep-dive loader — single source of truth for the
 * `/api/intelligence/v1/country-deep-dive` call from the
 * webview. Mirrors the discriminated-outcome shape of the
 * other intel loaders.
 */

/** One ACLED actor row scoped to the requested country. */
export interface ActorRow {
  name: string;
  events: number;
  totalFatalities: number;
}

/** One GDELT article scoped to the requested country. */
export interface ArticleRow {
  url: string;
  title: string;
  domain: string;
  language: string;
  /** GDELT `YYYYMMDDTHHMMSSZ` timestamp. */
  seenDate: string;
}

/** One Telegram message whose body mentions the country. */
export interface TelegramRow {
  channel: string;
  dataPost: string;
  url: string;
  /** ISO-8601 timestamp. */
  datetime: string;
  text: string;
  views: string;
}

/** Aggregated totals returned alongside the per-section rows. */
export interface CountryTotals {
  events: number;
  incidents: number;
  messages: number;
}

/** Wire-format response — mirrors `CountryDeepDiveResponse`. */
export interface CountryDeepDiveResponse {
  /** Echo of the requested country, canonicalised
   *  (Title Case + trimmed). */
  country: string;
  /** Coarse region label. */
  region: string;
  actors: ActorRow[];
  articles: ArticleRow[];
  telegram: TelegramRow[];
  totals: CountryTotals;
  assembledAtMs: number;
  stale: boolean;
}

/** Required + optional query knobs. */
export interface CountryDeepDiveQuery {
  /** Country name. Required — empty queries surface as a
   *  client-side `invalid_request` error envelope. */
  country: string;
  /** Cap on actors returned (server clamps to 1..100). */
  actors?: number;
  /** Cap on each feed (articles + telegram) — server clamps
   *  to 1..100. */
  limit?: number;
}

/** Discriminated outcome — never throws. */
export type CountryDeepDiveOutcome =
  | { kind: "ready"; response: CountryDeepDiveResponse }
  | {
      kind: "error";
      code: ErrorCode;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

/** Stable error codes. */
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
 * Fetch the country deep-dive payload. Never throws.
 *
 * Returns an `invalid_request` error outcome (without making a
 * network call) when `query.country` is empty / whitespace.
 */
export async function loadCountryDeepDive(
  query: CountryDeepDiveQuery,
  opts: LoadOptions = {},
): Promise<CountryDeepDiveOutcome> {
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
  if (typeof query.actors === "number" && Number.isFinite(query.actors)) {
    params.set("actors", String(Math.max(1, Math.floor(query.actors))));
  }
  if (typeof query.limit === "number" && Number.isFinite(query.limit)) {
    params.set("limit", String(Math.max(1, Math.floor(query.limit))));
  }
  const url = `${baseUrl}/api/intelligence/v1/country-deep-dive?${params.toString()}`;
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
      const body = (await resp.json()) as CountryDeepDiveResponse;
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
