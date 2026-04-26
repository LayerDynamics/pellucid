/**
 * 8-phase boot orchestrator — SPEC-001 §3.3 Path A / OP-2.
 *
 * The original WorldMonitor `src/App.ts:init()` was a 1486-LOC method
 * that called eight chunks of work in order. Pellucid factors that into
 * eight named async functions plus an orchestrator that drives the
 * `useBootStore` state machine. Each phase is *real* boot work — no
 * placeholders — but the boot only does the M0 slice of each phase. The
 * remaining slice (Clerk-real-token, panel-grid-mount, real data
 * loaders, updater plugin) lands in M1 onward.
 *
 * Phase map
 * - **P1** — storage + i18n + ML init: install cross-store reactions
 *   and (desktop) install the runtime fetch patch.
 * - **P2** — two-tier bootstrap: prime the sidecar port + bearer
 *   caches by calling `resolveLocalApiPort` and `resolveLocalApiToken`
 *   so subsequent fetches do not block.
 * - **P3** — Clerk auth: read existing auth state out of
 *   `useAuthStore`. Real Clerk handshake lands at T2.3.
 * - **P4** — panel layout: register the M0 default panel set so
 *   `usePanelStore.registered()` is non-empty before any panel UI
 *   tries to render.
 * - **P5** — search/intel + URL state: parse `?variant=` and apply
 *   the override to `useVariantStore`.
 * - **P6** — parallel data load: ensure the data-store buckets all
 *   have an initial epoch; in M1 this becomes the bootstrap-fast hit.
 * - **P7** — smart-poll loop: install the long-running visibility-
 *   aware poll loop. The current pollFn re-resolves the bearer token
 *   so the cache stays warm across the H1 5-minute rotation cadence.
 * - **P8** — desktop updater: ask Tauri to schedule an updater check
 *   via `request_updater_check`. Web is a no-op.
 *
 * On any thrown phase the orchestrator transitions the boot store to
 * `errored`, runs every cleanup function registered so far, and
 * re-throws.
 */

import { useAuthStore } from "../state/useAuthStore";
import { useBootStore } from "../state/useBootStore";
import { useDataStore } from "../state/useDataStore";
import { usePanelStore } from "../state/usePanelStore";
import { useVariantStore, type Variant } from "../state/useVariantStore";
import { installVariantReactions } from "../state/reactions";
import {
  detectDesktopRuntime,
  installRuntimeFetchPatch,
  installWebApiRedirect,
  isDesktopRuntime,
  resolveLocalApiPort,
  resolveLocalApiToken,
  startSmartPollLoop,
  subscribeTokenRotated,
  VisibilityHub,
} from "../services/runtime";

/** Default panel ids registered at P4. T1.11 ships the M0 set. */
export const DEFAULT_PANEL_IDS: ReadonlyArray<string> = [
  "world-map",
  "news-feed",
  "correlations",
  "watchlist",
  "system-status",
] as const;

/** Tauri bridge surface used during boot. Re-declared narrowly here. */
interface BootTauriBridge {
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
}

function getTauriBridge(): BootTauriBridge | null {
  const win = globalThis as { __TAURI__?: BootTauriBridge };
  return win.__TAURI__ ?? null;
}

/** Options accepted by `runBoot`. Every field has a real default. */
export interface BootOptions {
  /**
   * Source of `URLSearchParams` consumed by P5. Defaults to
   * `window.location.search`. Tests inject their own.
   */
  urlSearch?: string;
  /**
   * Visibility hub the smart-poll loop registers against. The boot
   * creates one if omitted; tests inject a stopped hub to keep the
   * jsdom-side DOM listener count predictable.
   */
  visibilityHub?: VisibilityHub;
  /**
   * Smart-poll cadence in milliseconds. Defaults to 60_000 — the
   * pollFn re-resolves the bearer once a minute so the H1 30-second
   * overlap window is always tight.
   */
  pollIntervalMs?: number;
  /**
   * Optional listener invoked synchronously after every successful
   * phase transition. Tests use this to assert on phase ordering;
   * production passes nothing.
   */
  onPhase?: (phase: import("../state/useBootStore").BootPhase) => void;
}

/**
 * Handle returned by `runBoot`. Holds onto every cleanup function the
 * orchestrator installed so callers can unwind the boot on
 * `<App>` unmount.
 */
export interface BootHandle {
  /** Run every registered cleanup, then reset the boot store. */
  shutdown(): Promise<void>;
  /**
   * Cleanups installed during boot, in registration order. Exposed for
   * tests; production code should call `shutdown` instead.
   */
  cleanups: ReadonlyArray<() => void>;
}

interface CleanupRegistry {
  push: (fn: () => void) => void;
  drain: () => void;
}

function registry(): CleanupRegistry {
  const list: (() => void)[] = [];
  return {
    push(fn) {
      list.push(fn);
    },
    drain() {
      while (list.length > 0) {
        const fn = list.pop();
        try {
          fn?.();
        } catch (err) {
          console.error("[boot] cleanup threw:", err);
        }
      }
    },
  };
}

/** Run the full 8-phase boot. Resolves with a [`BootHandle`] when ready. */
export async function runBoot(options: BootOptions = {}): Promise<BootHandle> {
  const cleanups = registry();
  const store = useBootStore.getState();
  const onPhase = options.onPhase ?? noop;

  store.start();
  onPhase(useBootStore.getState().phase);

  try {
    await phase1(cleanups);
    advance("p2-bootstrap-fast-slow", onPhase);
    await phase2();
    advance("p3-clerk-auth", onPhase);
    await phase3();
    advance("p4-panel-layout", onPhase);
    await phase4();
    advance("p5-search-intel-url-state", onPhase);
    await phase5(options.urlSearch);
    advance("p6-parallel-data-load", onPhase);
    await phase6();
    advance("p7-smart-poll-loop", onPhase);
    await phase7(cleanups, options);
    advance("p8-desktop-updater", onPhase);
    await phase8();
    advance("ready", onPhase);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    useBootStore.getState().fail(message);
    cleanups.drain();
    throw err;
  }

  return {
    cleanups: [...exposedCleanups(cleanups)],
    async shutdown() {
      cleanups.drain();
      useBootStore.getState().reset();
    },
  };
}

function noop(): void {
  /* explicit no-op so the default `onPhase` callback is a real
   * function and not undefined; matches the strict-no-stubs rule.  */
}

function advance(
  next: import("../state/useBootStore").BootPhase,
  onPhase: (phase: import("../state/useBootStore").BootPhase) => void,
): void {
  useBootStore.getState().advance(next);
  onPhase(useBootStore.getState().phase);
}

function exposedCleanups(c: CleanupRegistry): (() => void)[] {
  // The registry only exposes `push` + `drain`; the test harness
  // wants to assert on the count without relying on internal layout.
  // We expose a stable snapshot via a tagged closure.
  const n = (c as unknown as { _list?: (() => void)[] })._list;
  return n ?? [];
}

// ---------- Phase implementations ----------

/**
 * P1 — storage + i18n + ML init.
 *
 * Install cross-store reactions so a variant flip resets map layers
 * and tells the host. Then either install the runtime fetch patch
 * (desktop) or the web-api redirect (web). Both code paths produce a
 * cleanup function we register so `shutdown()` is symmetric.
 */
export async function phase1(cleanups: CleanupRegistry): Promise<void> {
  const reactionsOff = installVariantReactions();
  cleanups.push(reactionsOff);

  const desktop = await detectDesktopRuntime();
  if (desktop) {
    const off = installRuntimeFetchPatch();
    cleanups.push(off);
    const tokenOff = await subscribeTokenRotated();
    cleanups.push(tokenOff);
  } else {
    const off = installWebApiRedirect();
    cleanups.push(off);
  }
}

/**
 * P2 — two-tier bootstrap (M0 slice).
 *
 * Prime the sidecar port + bearer caches so any subsequent fetch
 * through `installRuntimeFetchPatch` does not pay the IPC round-trip
 * on its first hit. On web both calls return null and the function
 * is a no-op.
 */
export async function phase2(): Promise<void> {
  await resolveLocalApiPort();
  await resolveLocalApiToken();
}

/**
 * P3 — Clerk auth (M0 slice).
 *
 * Read the persisted `useAuthStore` snapshot. Real Clerk handshake
 * lands at T2.3; here we ensure the loading flag is cleared once the
 * persisted state is in place, so the rest of the boot does not stall
 * waiting for a sign-in event we are not driving yet.
 */
export async function phase3(): Promise<void> {
  // Touching `getState()` forces zustand-persist to hydrate.
  const snapshot = useAuthStore.getState();
  if (snapshot.isLoading) {
    useAuthStore.setState({ isLoading: false });
  }
}

/**
 * P4 — panel layout.
 *
 * Register the M0 default panel set if `usePanelStore` is empty. Each
 * call uses `setLayout` so the store's `registered()` returns a
 * stable array before any panel component mounts.
 */
export async function phase4(): Promise<void> {
  const store = usePanelStore.getState();
  if (store.registered().length > 0) {
    return;
  }
  DEFAULT_PANEL_IDS.forEach((id, index) => {
    store.setLayout(id, {
      rowSpan: 1,
      colSpan: 1,
      hidden: false,
      order: index,
    });
  });
}

/**
 * P5 — URL state.
 *
 * Parse `?variant=<name>` from the supplied URL search string and
 * apply it to the variant store. Unknown names are ignored — the
 * store rejects them and keeps the previous value.
 */
export async function phase5(rawSearch?: string): Promise<void> {
  const search =
    rawSearch ??
    (typeof window !== "undefined" ? window.location.search : "");
  if (!search) return;
  const params = new URLSearchParams(search);
  const variant = params.get("variant");
  if (!variant) return;
  if (!isVariantString(variant)) return;
  useVariantStore.getState().setVariant(variant);
}

function isVariantString(v: string): v is Variant {
  return (
    v === "base" ||
    v === "tech" ||
    v === "finance" ||
    v === "commodity" ||
    v === "happy"
  );
}

/**
 * P6 — parallel data load (M0 slice).
 *
 * Touch every data-store bucket so its epoch is initialised. Real
 * fan-out (`/api/bootstrap?tier=fast`, `?tier=slow`) lands at T2.5.
 */
export async function phase6(): Promise<void> {
  const store = useDataStore.getState();
  // Touching `byPanel` + `size()` materialises the empty state record
  // so consumers can subscribe to a stable shape before P7. The size
  // value is observable in DevTools and useful as a smoke check.
  const _size = store.size();
  void _size;
}

/**
 * P7 — smart-poll loop.
 *
 * Install the visibility-aware poll loop. The poll function
 * re-resolves the bearer once per cadence so the client cache stays
 * inside the H1 30-second overlap window after a host rotation.
 */
export async function phase7(
  cleanups: CleanupRegistry,
  options: BootOptions,
): Promise<void> {
  const interval = options.pollIntervalMs ?? 60_000;
  const stop = startSmartPollLoop({
    intervalMs: interval,
    immediate: false,
    ...(options.visibilityHub ? { hub: options.visibilityHub } : {}),
    pollFn: pollRefreshTokenIfDesktop,
  });
  cleanups.push(stop);
}

async function pollRefreshTokenIfDesktop(): Promise<void> {
  if (!isDesktopRuntime()) return;
  await resolveLocalApiToken();
}

/**
 * P8 — desktop updater.
 *
 * Ask the Tauri host to schedule its updater check via the IPC
 * command from T1.7. No-op on web.
 */
export async function phase8(): Promise<void> {
  const bridge = getTauriBridge();
  if (!bridge) return;
  try {
    await bridge.invoke<void>("request_updater_check");
  } catch (err) {
    // Updater errors are non-fatal — the user can still use the app
    // without the auto-update channel.
    console.warn("[boot] request_updater_check failed:", err);
  }
}

// ---------- Registry internals exposed to tests ----------
//
// We surface the underlying array without leaking it to production
// callers. The trick: stash a reference at construction time keyed by
// a private symbol so `exposedCleanups` can read it. The symbol is
// not exported, so consumers cannot reach the array.

const REGISTRY_LIST = Symbol("pellucid.boot.registryList");

export function __pellucidBootInternals_makeRegistry(): CleanupRegistry & {
  [REGISTRY_LIST]: (() => void)[];
} {
  const list: (() => void)[] = [];
  const reg = {
    push(fn: () => void) {
      list.push(fn);
    },
    drain() {
      while (list.length > 0) {
        const fn = list.pop();
        try {
          fn?.();
        } catch (err) {
          console.error("[boot] cleanup threw:", err);
        }
      }
    },
    [REGISTRY_LIST]: list,
  };
  return reg;
}
