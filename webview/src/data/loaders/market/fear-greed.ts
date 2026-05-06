/**
 * Fear/greed loader — wraps `/api/market/v1/fear-greed`.
 *
 * Mirrors the wire shape of
 * `crates/pellucid-handlers/src/market/v1/fear_greed.rs`. Same
 * discriminated outcome shape every market loader uses; the
 * panel branches exhaustively on `outcome.kind`.
 */

/** Bucket label — mirrors the Rust `SentimentLabel` enum's
 *  serde `kebab-case` rename. */
export type SentimentLabel =
  | "extreme-fear"
  | "fear"
  | "neutral"
  | "greed"
  | "extreme-greed";

/** One sub-index in the response. */
export interface Component {
  /** Stable component id (`volatility` / `momentum` / `strength` /
   *  `volume`). */
  name: string;
  /** 0–100 score. */
  score: number;
  /** Label bucket for this component alone. */
  label: SentimentLabel;
  /** Short rationale string the panel renders next to the chip. */
  rationale: string;
}

/** Wire-format response. */
export interface FearGreedResponse {
  /** Composite 0–100 score (unweighted mean of components). */
  score: number;
  /** Composite bucket label. */
  label: SentimentLabel;
  /** Sub-index components present in this response. */
  components: Component[];
  assembledAtMs: number;
  stale: boolean;
}

/** Discriminated outcome — never throws. */
export type FearGreedOutcome =
  | { kind: "ready"; response: FearGreedResponse }
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
 * Fetch the composite fear/greed snapshot. Never throws.
 */
export async function loadFearGreed(
  opts: LoadOptions = {},
): Promise<FearGreedOutcome> {
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;
  const url = `${baseUrl}/api/market/v1/fear-greed`;
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
      const body = (await resp.json()) as FearGreedResponse;
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
