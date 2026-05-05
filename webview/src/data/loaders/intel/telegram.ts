/**
 * Telegram intelligence loader — single source of truth for the
 * `/api/telegram/v1/feed` call from the webview.
 *
 * Mirrors the discriminated-outcome shape of
 * `loaders/intel/gdelt.ts` so the panel branches exhaustively
 * on `outcome.kind`. Wire types match `FeedResponse` in
 * `crates/pellucid-handlers/src/telegram/v1/feed.rs` 1:1
 * (camelCase on both sides).
 */

/** One message row in the wire response. Mirrors
 *  `TelegramMessage` in the Rust handler. */
export interface TelegramMessage {
  /** Channel slug (without leading @). */
  channel: string;
  /** `<channel>/<id>` data-post identifier. */
  dataPost: string;
  /** Permalink. */
  url: string;
  /** ISO-8601 upstream timestamp. */
  datetime: string;
  /** Plain-text message body. */
  text: string;
  /** View counter as the upstream rendered it (`"12.4K"` etc.). */
  views: string;
}

/** Wire-format response. Mirrors `FeedResponse`. */
export interface FeedResponse {
  rows: TelegramMessage[];
  /** Echo of the seeder's channel basket. */
  channels: string[];
  /** Wall-clock ms when the seeder assembled the snapshot. */
  assembledAtMs: number;
  /** Total rows before the limit clamp. */
  total: number;
  /** True when the response was synthesised from a stale cache row. */
  stale: boolean;
}

/** Optional query knobs. */
export interface FeedQuery {
  /** Cap on rows. Server clamps to [1, 200]. */
  limit?: number;
  /** Filter to a single channel slug (case-insensitive,
   *  trim-tolerant on the server). */
  channel?: string;
}

/** Discriminated outcome — never throws, always returns one. */
export type FeedOutcome =
  | { kind: "ready"; response: FeedResponse }
  | {
      kind: "error";
      code: ErrorCode;
      message: string;
      httpStatus: number;
      retryAfterSecs: number | null;
    };

/** Stable error codes — same set as the news + gdelt loaders. */
export type ErrorCode =
  | "invalid_request"
  | "cache_failure"
  | "cache_shape"
  | "bootstrap_upstream_empty"
  | "entitlement_forbidden"
  | "clerk_unauthorized"
  | "network"
  | "unknown";

/** Loader options — DI hooks for tests + base URL override. */
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
 * Fetch the Telegram intel feed. Never throws.
 */
export async function loadTelegramFeed(
  q: FeedQuery = {},
  opts: LoadOptions = {},
): Promise<FeedOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const params = new URLSearchParams();
  if (typeof q.limit === "number" && Number.isFinite(q.limit)) {
    params.set("limit", String(Math.max(1, Math.floor(q.limit))));
  }
  if (q.channel && q.channel.trim().length > 0) {
    params.set("channel", q.channel.trim());
  }
  const qs = params.toString();
  const url = `${baseUrl}/api/telegram/v1/feed${qs ? `?${qs}` : ""}`;
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
      const body = (await resp.json()) as FeedResponse;
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

/**
 * Parse an ISO-8601 datetime string into wall-clock ms. Returns
 * `null` when the input doesn't parse. Exported so the panel can
 * render the message timestamp via `NewsCard`'s relative-time
 * formatter without re-implementing the parser.
 */
export function parseIsoDatetime(datetime: string): number | null {
  if (typeof datetime !== "string" || datetime.length === 0) return null;
  const ms = Date.parse(datetime);
  return Number.isFinite(ms) ? ms : null;
}
