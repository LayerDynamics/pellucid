/**
 * Bootstrap (cold-start hydration) service — port of the original
 * WorldMonitor `getHydratedData` / `markBootstrapAsLive` /
 * `getBootstrapHydrationState` / `fetchBootstrapData` quartet from
 * `src/services/bootstrap.ts:1-...` (referenced by SPEC-001 §3.3).
 *
 * The two-tier hydration sequence (OP-4):
 *   1. P2 of the boot machine fires `fetchBootstrapData("fast")`
 *      with a 3-second budget. The 67 fast keys land first so the
 *      above-the-fold panels can paint.
 *   2. Concurrently, `fetchBootstrapData("slow")` runs with a
 *      5-second budget for the 45 lower-cadence keys.
 *   3. Each tier has its own `AbortController` so a stalled slow
 *      tier never blocks the fast paint, and a budget-elapsed
 *      tier surfaces as a marked-stale state instead of a thrown
 *      promise.
 *
 * The state machine (`HydrationState`) is consumed by panel
 * components: panels gate render on `state.hasFast` (live) and
 * surface a "loading more context" badge while `state.hasSlow`
 * is `false`.
 */

const DEFAULT_FAST_BUDGET_MS = 3_000;
const DEFAULT_SLOW_BUDGET_MS = 5_000;

/** Two-tier hydration state, observed by panels + boot machine. */
export interface HydrationState {
  /** `true` once the fast tier has produced a 200 (any data). */
  hasFast: boolean;
  /** `true` once the slow tier has produced a 200. */
  hasSlow: boolean;
  /** `true` once `markBootstrapAsLive()` has fired (i.e. the
   *  webview has stopped serving the cold-start envelope and is
   *  now reading from the live cache). Mirrors the WorldMonitor
   *  flag of the same name. */
  isLive: boolean;
  /** Per-tier outcome. `null` while pending. */
  fastOutcome: TierOutcome | null;
  /** Per-tier outcome. `null` while pending. */
  slowOutcome: TierOutcome | null;
}

/** What a single tier fetch resolved to. */
export interface TierOutcome {
  /** HTTP status from the gateway. */
  status: number;
  /** `data: { key → value }` from the bootstrap envelope. */
  data: Record<string, unknown>;
  /** `missing` array from the bootstrap envelope. */
  missing: readonly string[];
  /** `negative` array from the bootstrap envelope. */
  negative: readonly string[];
  /** Wall-clock duration of the fetch, ms. */
  elapsedMs: number;
  /** `true` iff the fetch was aborted because the budget elapsed. */
  budgetExceeded: boolean;
  /** `error.code` from the gateway envelope on a 4xx/5xx; null on success. */
  errorCode: string | null;
  /** `error.retry_after_secs` on the M4 outage path; null otherwise. */
  retryAfterSecs: number | null;
}

/** Wire shape returned by `GET /api/bootstrap/v1/get`. */
interface BootstrapEnvelope {
  data: Record<string, unknown>;
  missing: string[];
  negative: string[];
}

interface BootstrapErrorEnvelope {
  error: {
    code: string;
    message: string;
    retry_after_secs?: number;
    requested?: number;
  };
}

const initialState = (): HydrationState => ({
  hasFast: false,
  hasSlow: false,
  isLive: false,
  fastOutcome: null,
  slowOutcome: null,
});

let state: HydrationState = initialState();

/** Reset hydration state — used by tests + by an explicit
 *  full-app reload sequence. */
export function resetBootstrapHydrationState(): void {
  state = initialState();
}

/** Snapshot the current hydration state. Returned object is a
 *  shallow copy so callers cannot mutate the shared singleton. */
export function getBootstrapHydrationState(): HydrationState {
  return {
    ...state,
    fastOutcome: state.fastOutcome
      ? { ...state.fastOutcome, missing: [...state.fastOutcome.missing], negative: [...state.fastOutcome.negative] }
      : null,
    slowOutcome: state.slowOutcome
      ? { ...state.slowOutcome, missing: [...state.slowOutcome.missing], negative: [...state.slowOutcome.negative] }
      : null,
  };
}

/** Flip the live flag. Called by the smart-poll loop once the
 *  first round of live polls has completed. */
export function markBootstrapAsLive(): void {
  state.isLive = true;
}

/** Read a hydrated value out of either tier's outcome. Returns
 *  `undefined` if the key was never requested or was in `missing`. */
export function getHydratedData<T = unknown>(key: string): T | undefined {
  const fast = state.fastOutcome?.data?.[key] as T | undefined;
  if (fast !== undefined) return fast;
  return state.slowOutcome?.data?.[key] as T | undefined;
}

/** Options for [`fetchBootstrapData`]. */
export interface FetchOptions {
  /** Base URL of the edge bin / sidecar. Defaults to `""`
   *  (same-origin). */
  baseUrl?: string;
  /** Per-tier budget. Defaults to 3000ms (fast) / 5000ms (slow). */
  budgetMs?: number;
  /** Inject the fetch implementation (testing). Defaults to
   *  `globalThis.fetch`. */
  fetchImpl?: typeof fetch;
  /** External AbortController to thread through. Tests use this to
   *  cancel from outside. The internal budget timer creates its own
   *  signal which is composed via `AbortSignal.any` when supported. */
  signal?: AbortSignal;
}

/**
 * Fetch the named bootstrap tier and update the singleton state.
 * Returns the outcome so callers can branch on it without a state
 * read.
 */
export async function fetchBootstrapData(
  tier: "fast" | "slow",
  opts: FetchOptions = {},
): Promise<TierOutcome> {
  const budgetMs =
    opts.budgetMs ??
    (tier === "fast" ? DEFAULT_FAST_BUDGET_MS : DEFAULT_SLOW_BUDGET_MS);
  const baseUrl = opts.baseUrl ?? "";
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch;

  const internalAbort = new AbortController();
  const timer = setTimeout(() => internalAbort.abort(), budgetMs);
  const signal = composeSignals(internalAbort, opts.signal);

  const startedAt = nowMs();
  let outcome: TierOutcome;
  try {
    const resp = await fetchImpl(
      `${baseUrl}/api/bootstrap/v1/get?tier=${tier}`,
      { signal, method: "GET" },
    );
    const elapsedMs = nowMs() - startedAt;
    if (resp.status === 200) {
      const env = (await resp.json()) as BootstrapEnvelope;
      outcome = {
        status: 200,
        data: env.data ?? {},
        missing: env.missing ?? [],
        negative: env.negative ?? [],
        elapsedMs,
        budgetExceeded: false,
        errorCode: null,
        retryAfterSecs: null,
      };
    } else {
      const err = (await resp.json().catch(() => ({}))) as Partial<BootstrapErrorEnvelope>;
      outcome = {
        status: resp.status,
        data: {},
        missing: [],
        negative: [],
        elapsedMs,
        budgetExceeded: false,
        errorCode: err?.error?.code ?? null,
        retryAfterSecs: err?.error?.retry_after_secs ?? null,
      };
    }
  } catch (e) {
    const elapsedMs = nowMs() - startedAt;
    const aborted = (e as { name?: string })?.name === "AbortError";
    outcome = {
      status: 0,
      data: {},
      missing: [],
      negative: [],
      elapsedMs,
      budgetExceeded: aborted && elapsedMs >= budgetMs,
      errorCode: aborted ? "abort" : "network",
      retryAfterSecs: null,
    };
  } finally {
    clearTimeout(timer);
  }

  if (tier === "fast") {
    state.fastOutcome = outcome;
    state.hasFast = outcome.status === 200;
  } else {
    state.slowOutcome = outcome;
    state.hasSlow = outcome.status === 200;
  }
  return outcome;
}

/** Compose an internal controller's signal with an optional
 *  external signal. Falls back to plain internal signal when
 *  `AbortSignal.any` is unavailable (older runtimes). The
 *  internal *controller* is required (not just its signal) so the
 *  fallback path can `.abort()` on external abort. */
function composeSignals(
  internal: AbortController,
  external?: AbortSignal,
): AbortSignal {
  if (!external) return internal.signal;
  // Bun + modern Node + Chrome ≥ 116 ship AbortSignal.any.
  const anyImpl = (AbortSignal as unknown as {
    any?: (sigs: AbortSignal[]) => AbortSignal;
  }).any;
  if (typeof anyImpl === "function")
    return anyImpl([internal.signal, external]);
  // Manual fan-in fallback.
  if (external.aborted) {
    internal.abort();
    return internal.signal;
  }
  external.addEventListener("abort", () => internal.abort(), { once: true });
  return internal.signal;
}

function nowMs(): number {
  // `performance.now()` is monotonic but returns elapsed-since-
  // navigation; for a wall-clock duration we just need the delta,
  // and `performance` is universally available in our targets
  // (Bun, modern browsers, Node ≥ 16 via `perf_hooks`).
  if (typeof performance !== "undefined" && typeof performance.now === "function") {
    return performance.now();
  }
  return Date.now();
}
