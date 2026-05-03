/**
 * Aviation data loader — single source of truth for the
 * `/api/aviation/v1/get-flight-status` call from the webview.
 *
 * Direct port of `src/data/loaders/aviation.ts` from the original
 * WorldMonitor codebase. Panels never call `fetch()` directly;
 * they call this loader so:
 *   1. Cache-key + tier semantics live in one place.
 *   2. Error envelopes are normalised before reaching React.
 *   3. The tier-locked path can be exercised in isolation.
 *
 * The loader returns a discriminated union, never throws on
 * upstream failure — panels branch on `outcome.kind`.
 */

/** The shape the gateway returns on 200. Mirrors the canonical
 *  `crates/pellucid-handlers/src/generated/aviation/v1/
 *  get_flight_status.rs::FlightStatus` struct. The sebuf TS
 *  emitter (T2.5 carry-forward) will eventually generate this
 *  alongside the Rust types; until then we hand-author here. */
export interface FlightStatus {
  flight: string;
  scheduled_departure: string;
  scheduled_arrival: string;
  status: string;
  origin: string;
  destination: string;
  departure_gate?: string;
  arrival_gate?: string;
}

/** Inputs the panel passes to the loader. */
export interface FlightStatusQuery {
  /** IATA flight number, e.g. `"AA100"`. Case-insensitive — the
   *  handler uppercases before keying the cache. */
  flight: string;
  /** ISO 8601 date (`YYYY-MM-DD`). */
  date: string;
  /** 3-letter IATA airport code, e.g. `"JFK"`. */
  origin: string;
}

/** Loader outcome. Discriminated by `kind` so React can branch
 *  exhaustively. */
export type FlightStatusOutcome =
  | { kind: "ready"; status: FlightStatus }
  | { kind: "error"; code: ErrorCode; message: string; httpStatus: number; retryAfterSecs: number | null };

/** Stable error codes the panel branches on. Mirrors the
 *  handler's `HandlerError::code()` (`crates/pellucid-handlers/src/
 *  aviation/v1/get_flight_status.rs`). */
export type ErrorCode =
  | "invalid_request"
  | "upstream_failure"
  | "cache_failure"
  | "entitlement_forbidden"
  | "clerk_unauthorized"
  | "network"
  | "unknown";

/** Loader options — mostly for testing (fetch injection + base
 *  URL override). */
export interface LoadOptions {
  /** Base URL of the edge bin / sidecar. Defaults to `""`
   *  (same-origin). */
  baseUrl?: string;
  /** Inject the fetch implementation. Defaults to
   *  `globalThis.fetch`. */
  fetchImpl?: typeof fetch;
  /** Optional caller-side abort. */
  signal?: AbortSignal;
  /** Optional Clerk session token to forward as `Authorization:
   *  Bearer ...`. The aviation route is anonymous in M0 but the
   *  loader honours the token so the same code path serves
   *  tier-gated panels (T2.11 demonstrates the wiring). */
  bearerToken?: string;
}

const ERROR_CODES = new Set<string>([
  "invalid_request",
  "upstream_failure",
  "cache_failure",
  "entitlement_forbidden",
  "clerk_unauthorized",
]);

/**
 * Fetch a flight status. Never throws; on any failure (network,
 * 4xx, 5xx, malformed body) returns `{ kind: "error", ... }`.
 */
export async function loadFlightStatus(
  q: FlightStatusQuery,
  opts: LoadOptions = {},
): Promise<FlightStatusOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const url = `${baseUrl}/api/aviation/v1/get-flight-status?flight=${encodeURIComponent(
    q.flight,
  )}&date=${encodeURIComponent(q.date)}&origin=${encodeURIComponent(q.origin)}`;
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
      const body = (await resp.json()) as FlightStatus;
      return { kind: "ready", status: body };
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

  // Error path — try to read the gateway envelope. If parsing the
  // error body fails too, fall back to a synthesised envelope so
  // the panel still has something to render.
  let envelope: { error?: { code?: string; message?: string } } = {};
  try {
    envelope = (await resp.json()) as typeof envelope;
  } catch {
    /* fall through with empty envelope */
  }
  const rawCode = envelope?.error?.code ?? null;
  const code: ErrorCode = rawCode && ERROR_CODES.has(rawCode) ? (rawCode as ErrorCode) : "unknown";
  const message = envelope?.error?.message ?? `HTTP ${httpStatus}`;
  return { kind: "error", code, message, httpStatus, retryAfterSecs: Number.isFinite(retryAfterSecs) ? retryAfterSecs : null };
}
