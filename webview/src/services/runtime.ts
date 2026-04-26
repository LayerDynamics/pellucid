/**
 * Runtime helpers — port of `src/services/runtime.ts:30-749` from the
 * original WorldMonitor app, modernised for Tauri 2 and the Pellucid
 * H1 token-rotation contract.
 *
 * This module is the single seam through which the webview talks to the
 * sidecar. It owns:
 *
 * - Tauri-runtime detection (`__TAURI__` global probe).
 * - Sidecar port + bearer token resolution via the IPC commands from
 *   T1.7 (`get_local_api_port`, `get_local_api_token`,
 *   `refresh_secrets`).
 * - URL building (`getApiBaseUrl`, `toApiUrl`) for both desktop and web
 *   targets.
 * - Two fetch interceptors: `installRuntimeFetchPatch` (desktop) routes
 *   `/api/*` through the sidecar with bearer attached; the matching
 *   `installWebApiRedirect` (web) sends `/api/*` to the public WM API.
 * - `VisibilityHub` + `startSmartPollLoop` — page-visibility-aware
 *   polling that pauses while the tab is hidden so the host CPU stays
 *   quiet.
 *
 * Everything that holds state exposes a `__pellucidRuntimeInternals`
 * reset hook so unit tests can keep their fixtures isolated.
 */

import { useVariantStore } from "../state/useVariantStore";

// ---------- constants ----------

/** Public WorldMonitor API base used in web target builds. */
export const WEB_API_BASE = "https://api.worldmonitor.app";

/** The Tauri event the host emits after a successful T1.8 rotation. */
export const TOKEN_ROTATED_EVENT = "token_rotated";

const FETCH_PATCH_FLAG = Symbol.for("pellucid.runtime.fetchPatched");
const REDIRECT_FLAG = Symbol.for("pellucid.runtime.webRedirectInstalled");

// ---------- Tauri bridge surface ----------

/**
 * Minimal shape of `window.__TAURI__` we depend on. The real Tauri
 * bridge surfaces many more methods; we only typecheck what we use.
 */
interface TauriBridge {
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
  event?: {
    listen: <P>(
      name: string,
      cb: (event: { payload: P }) => void,
    ) => Promise<() => void>;
  };
}

/**
 * Snapshot returned by `refresh_secrets` (matches `SecretBundle` in
 * `crates/pellucid-tauri/src/ipc.rs`). Optional fields are forwarded
 * verbatim so the runtime never lies about what the host returned.
 */
interface SecretBundle {
  sidecar_token: string | null;
  sidecar_token_previous: string | null;
}

function getTauriBridge(): TauriBridge | null {
  if (typeof globalThis === "undefined") {
    return null;
  }
  const win = globalThis as { __TAURI__?: TauriBridge };
  return win.__TAURI__ ?? null;
}

// ---------- module-private cached state ----------

interface RuntimeCache {
  desktop: boolean | null;
  port: number | null;
  current: string | null;
  previous: string | null;
  rotateUnsubscribe: (() => void) | null;
}

const cache: RuntimeCache = {
  desktop: null,
  port: null,
  current: null,
  previous: null,
  rotateUnsubscribe: null,
};

// ---------- desktop detection ----------

/**
 * Asynchronously detect whether the page is running inside the Tauri
 * webview. Caches the result; subsequent callers see the cached value
 * without re-probing.
 */
export async function detectDesktopRuntime(): Promise<boolean> {
  if (cache.desktop !== null) {
    return cache.desktop;
  }
  const bridge = getTauriBridge();
  if (!bridge) {
    cache.desktop = false;
    return false;
  }
  try {
    // Probe with a cheap IPC call; the response value is irrelevant —
    // we only care that the call resolves.
    await bridge.invoke<unknown>("get_variant");
    cache.desktop = true;
  } catch {
    cache.desktop = false;
  }
  return cache.desktop;
}

/**
 * Synchronous form. Returns `true` iff `__TAURI__` is on the global
 * (does not call IPC). Cheap enough to call on every fetch.
 */
export function isDesktopRuntime(): boolean {
  return getTauriBridge() !== null;
}

// ---------- port + token resolution ----------

/**
 * Ask the host for the sidecar port and cache the answer. Returns
 * `null` on web builds or if IPC fails.
 */
export async function resolveLocalApiPort(): Promise<number | null> {
  if (cache.port !== null) {
    return cache.port;
  }
  const bridge = getTauriBridge();
  if (!bridge) {
    return null;
  }
  try {
    const port = await bridge.invoke<number>("get_local_api_port");
    if (typeof port === "number" && Number.isFinite(port) && port > 0) {
      cache.port = port;
      return port;
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * Synchronous accessor — returns the cached port from a prior
 * `resolveLocalApiPort` call, or `null` if it has not run yet.
 */
export function getLocalApiPort(): number | null {
  return cache.port;
}

/**
 * Pull the consolidated bearer pair from the host via
 * `refresh_secrets`. Caches both current + previous so the fetch patch
 * can attach the right header without an extra IPC round-trip.
 */
export async function resolveLocalApiToken(): Promise<string | null> {
  const bridge = getTauriBridge();
  if (!bridge) {
    return null;
  }
  try {
    const bundle = await bridge.invoke<SecretBundle>("refresh_secrets");
    cache.current = bundle.sidecar_token;
    cache.previous = bundle.sidecar_token_previous;
    return cache.current;
  } catch {
    return null;
  }
}

/** Synchronous accessor — returns the cached current token. */
export function getLocalApiToken(): string | null {
  return cache.current;
}

/** Cached previous token (still inside the H1 30-second overlap). */
export function getLocalApiTokenPrevious(): string | null {
  return cache.previous;
}

/**
 * Subscribe to the host's `token_rotated` Tauri event so the cache
 * stays fresh after an H1 rotation. Returns a cleanup function.
 */
export async function subscribeTokenRotated(): Promise<() => void> {
  const bridge = getTauriBridge();
  if (!bridge?.event) {
    return () => {};
  }
  if (cache.rotateUnsubscribe) {
    return cache.rotateUnsubscribe;
  }
  const unlisten = await bridge.event.listen<{ at_ms: number }>(
    TOKEN_ROTATED_EVENT,
    () => {
      void resolveLocalApiToken();
    },
  );
  cache.rotateUnsubscribe = () => {
    try {
      unlisten();
    } finally {
      cache.rotateUnsubscribe = null;
    }
  };
  return cache.rotateUnsubscribe;
}

// ---------- URL builders ----------

/**
 * Returns the API base URL the webview should target. Desktop reads
 * from the cached port; web returns the public host. Falls back to
 * the `VITE_TARGET` env var when neither cache nor `__TAURI__` is
 * present (i.e. during SSR / first paint before detection has run).
 */
export function getApiBaseUrl(): string {
  if (cache.port !== null && (cache.desktop ?? isDesktopRuntime())) {
    return `http://127.0.0.1:${cache.port}`;
  }
  if (isDesktopRuntime()) {
    // Desktop runtime present but port not yet resolved — return a
    // 127.0.0.1 origin without a port; callers should await
    // `resolveLocalApiPort` before fetching.
    return "http://127.0.0.1";
  }
  if (import.meta.env?.VITE_TARGET === "desktop") {
    return "http://127.0.0.1";
  }
  return WEB_API_BASE;
}

/**
 * Build a full URL for an API path. Accepts both `/api/...` and
 * `api/...` styles for ergonomic call sites.
 */
export function toApiUrl(path: string): string {
  const base = getApiBaseUrl();
  if (path.startsWith("http://") || path.startsWith("https://")) {
    return path;
  }
  const normalized = path.startsWith("/") ? path : `/${path}`;
  return `${base}${normalized}`;
}

// ---------- fetch patches ----------

interface FetchPatchOptions {
  /** Override for the bearer header name. Defaults to `Authorization`. */
  authHeader?: string;
}

interface PatchableGlobal {
  fetch: typeof fetch;
  [FETCH_PATCH_FLAG]?: typeof fetch;
  [REDIRECT_FLAG]?: typeof fetch;
}

/**
 * Wrap `globalThis.fetch` so any URL beginning with `/api` (or an
 * already-absolute desktop URL) is routed through the sidecar with the
 * cached bearer attached. Idempotent: calling twice does not
 * double-wrap.
 *
 * Returns an uninstall function that restores the previous `fetch`.
 */
export function installRuntimeFetchPatch(
  options: FetchPatchOptions = {},
): () => void {
  const authHeader = options.authHeader ?? "authorization";
  const target = globalThis as unknown as PatchableGlobal;
  if (target[FETCH_PATCH_FLAG]) {
    // Already installed — return a no-op cleanup so the caller can
    // still treat the return value uniformly.
    return () => {};
  }
  const original = target.fetch;
  target[FETCH_PATCH_FLAG] = original;

  const patched: typeof fetch = async (input, init) => {
    const url = inputToUrlString(input);
    if (!shouldRouteThroughSidecar(url)) {
      return original(input, init);
    }
    const port = cache.port ?? (await resolveLocalApiPort());
    const token = cache.current ?? (await resolveLocalApiToken());
    if (port === null) {
      return original(input, init);
    }
    const rewritten = rewriteForSidecar(url, port);
    const mergedInit = mergeAuth(init, authHeader, token);
    if (typeof input === "string") {
      return original(rewritten, mergedInit);
    }
    if (input instanceof URL) {
      return original(new URL(rewritten), mergedInit);
    }
    // Request object — re-wrap so we can swap URL + headers.
    return original(new Request(rewritten, input), mergedInit);
  };

  target.fetch = patched;
  return () => {
    if (target[FETCH_PATCH_FLAG] === original) {
      target.fetch = original;
      delete target[FETCH_PATCH_FLAG];
    }
  };
}

/**
 * Web-target counterpart to `installRuntimeFetchPatch`. Rewrites
 * relative `/api/*` URLs to the public WorldMonitor API base. No
 * bearer header is attached — auth on the public API rides on the
 * Clerk cookie / session.
 */
export function installWebApiRedirect(
  options: { baseUrl?: string } = {},
): () => void {
  const base = options.baseUrl ?? WEB_API_BASE;
  const target = globalThis as unknown as PatchableGlobal;
  if (target[REDIRECT_FLAG]) {
    return () => {};
  }
  const original = target.fetch;
  target[REDIRECT_FLAG] = original;

  const patched: typeof fetch = async (input, init) => {
    const url = inputToUrlString(input);
    if (!url.startsWith("/api/")) {
      return original(input, init);
    }
    const rewritten = `${base}${url}`;
    if (typeof input === "string") {
      return original(rewritten, init);
    }
    if (input instanceof URL) {
      return original(new URL(rewritten), init);
    }
    return original(new Request(rewritten, input), init);
  };

  target.fetch = patched;
  return () => {
    if (target[REDIRECT_FLAG] === original) {
      target.fetch = original;
      delete target[REDIRECT_FLAG];
    }
  };
}

function inputToUrlString(input: RequestInfo | URL): string {
  if (typeof input === "string") {
    return input;
  }
  if (input instanceof URL) {
    return input.toString();
  }
  return input.url;
}

function shouldRouteThroughSidecar(url: string): boolean {
  if (url.startsWith("/api/")) {
    return true;
  }
  if (url.startsWith("http://127.0.0.1/")) {
    return true;
  }
  return false;
}

function rewriteForSidecar(url: string, port: number): string {
  if (url.startsWith("/api/")) {
    return `http://127.0.0.1:${port}${url}`;
  }
  if (url.startsWith("http://127.0.0.1/")) {
    return url.replace("http://127.0.0.1/", `http://127.0.0.1:${port}/`);
  }
  return url;
}

function mergeAuth(
  init: RequestInit | undefined,
  header: string,
  token: string | null,
): RequestInit {
  if (!token) {
    return init ?? {};
  }
  const headers = new Headers(init?.headers);
  if (!headers.has(header)) {
    headers.set(header, `Bearer ${token}`);
  }
  return { ...(init ?? {}), headers };
}

// ---------- VisibilityHub ----------

/**
 * Listener invoked whenever the page-visibility state flips.
 * `visible = true` when `document.visibilityState === "visible"`.
 */
export type VisibilityListener = (visible: boolean) => void;

/**
 * Coordinates page-visibility subscribers so each component does not
 * have to attach its own `visibilitychange` handler. Constructed once
 * by `<App>` (T1.11) and passed through context.
 */
export class VisibilityHub {
  private readonly listeners = new Set<VisibilityListener>();
  private started = false;
  private readonly handler: () => void;

  constructor() {
    this.handler = () => {
      const visible = this.isVisible();
      for (const listener of this.listeners) {
        try {
          listener(visible);
        } catch (err) {
          // Swallow per-listener exceptions so one broken subscriber
          // does not silence the rest.
          console.error("[VisibilityHub] listener threw:", err);
        }
      }
    };
  }

  /** `true` iff `document.visibilityState === "visible"`. */
  isVisible(): boolean {
    if (typeof document === "undefined") {
      return true;
    }
    return document.visibilityState === "visible";
  }

  /** Attach the underlying DOM listener. Idempotent. */
  start(): void {
    if (this.started) return;
    if (typeof document === "undefined") return;
    document.addEventListener("visibilitychange", this.handler);
    this.started = true;
  }

  /** Detach the DOM listener and forget all subscribers. */
  stop(): void {
    if (typeof document !== "undefined" && this.started) {
      document.removeEventListener("visibilitychange", this.handler);
    }
    this.started = false;
    this.listeners.clear();
  }

  /**
   * Subscribe to visibility changes. Returns an unsubscribe function.
   * The listener is *not* invoked synchronously; first call happens on
   * the next visibility flip.
   */
  subscribe(listener: VisibilityListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** Number of attached subscribers. Useful for tests. */
  size(): number {
    return this.listeners.size;
  }
}

// ---------- smart-poll loop ----------

/** Options accepted by `startSmartPollLoop`. */
export interface SmartPollOptions {
  /** Interval between polls when the page is visible. */
  intervalMs: number;
  /** The work to perform on each tick. May return a promise. */
  pollFn: () => Promise<void> | void;
  /** Optional shared visibility hub. One is created if omitted. */
  hub?: VisibilityHub;
  /** Run `pollFn` once immediately on start. Defaults to `true`. */
  immediate?: boolean;
  /** Random jitter added to each interval (capped at the interval). */
  jitterMs?: number;
}

/**
 * Run `pollFn` every `intervalMs` while the page is visible; pause
 * while it is hidden; run once immediately when visibility resumes.
 * Returns a cleanup function.
 *
 * The loop is robust against `pollFn` rejection — exceptions are
 * caught and logged so a single bad tick does not stop the loop.
 */
export function startSmartPollLoop(opts: SmartPollOptions): () => void {
  if (opts.intervalMs <= 0) {
    throw new Error("startSmartPollLoop: intervalMs must be > 0");
  }
  const hub = opts.hub ?? new VisibilityHub();
  const ownsHub = !opts.hub;
  hub.start();

  const jitter = Math.max(0, Math.min(opts.jitterMs ?? 0, opts.intervalMs));
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const run = async () => {
    if (stopped) return;
    try {
      await opts.pollFn();
    } catch (err) {
      console.error("[smartPoll] tick rejected:", err);
    }
    if (stopped) return;
    if (!hub.isVisible()) {
      // The visibilitychange handler will reschedule when we resume.
      timer = null;
      return;
    }
    schedule();
  };

  const schedule = () => {
    if (stopped) return;
    const wait = opts.intervalMs + Math.floor(Math.random() * jitter);
    timer = setTimeout(() => {
      timer = null;
      void run();
    }, wait);
  };

  const visibilityListener: VisibilityListener = (visible) => {
    if (visible && timer === null && !stopped) {
      void run();
    }
  };
  const unsubscribe = hub.subscribe(visibilityListener);

  if (opts.immediate !== false) {
    void run();
  } else if (hub.isVisible()) {
    schedule();
  }

  return () => {
    stopped = true;
    unsubscribe();
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    if (ownsHub) {
      hub.stop();
    }
  };
}

// ---------- variant signal ----------

/**
 * Notify the host that the user picked a different visual variant.
 * Primarily a thin wrapper around `set_variant` — kept here so the
 * fetch interceptor and the variant store hit the same code path.
 */
export async function pushVariantToHost(
  variant: ReturnType<typeof useVariantStore.getState>["variant"],
): Promise<void> {
  const bridge = getTauriBridge();
  if (!bridge) return;
  try {
    await bridge.invoke<string>("set_variant", { variant });
  } catch (err) {
    console.warn("[runtime] set_variant failed:", err);
  }
}

// ---------- test internals ----------

/**
 * Test-only escape hatch. Exposed under a tilde-prefixed name so it is
 * obvious in autocomplete that production code should not call it.
 * Each unit test calls `__pellucidRuntimeInternals.reset()` in its
 * `afterEach` so module-private cache state never leaks.
 */
export const __pellucidRuntimeInternals = {
  reset(): void {
    cache.desktop = null;
    cache.port = null;
    cache.current = null;
    cache.previous = null;
    cache.rotateUnsubscribe?.();
    cache.rotateUnsubscribe = null;
    const target = globalThis as unknown as PatchableGlobal;
    if (target[FETCH_PATCH_FLAG]) {
      target.fetch = target[FETCH_PATCH_FLAG]!;
      delete target[FETCH_PATCH_FLAG];
    }
    if (target[REDIRECT_FLAG]) {
      target.fetch = target[REDIRECT_FLAG]!;
      delete target[REDIRECT_FLAG];
    }
  },
  setCachedPort(port: number | null): void {
    cache.port = port;
  },
  setCachedToken(current: string | null, previous: string | null = null): void {
    cache.current = current;
    cache.previous = previous;
  },
  setCachedDesktop(value: boolean | null): void {
    cache.desktop = value;
  },
  installFakeBridge(bridge: TauriBridge | null): () => void {
    const win = globalThis as { __TAURI__?: TauriBridge | null };
    const previous = win.__TAURI__;
    if (bridge === null) {
      delete (win as { __TAURI__?: TauriBridge | null }).__TAURI__;
    } else {
      win.__TAURI__ = bridge;
    }
    return () => {
      if (previous === undefined) {
        delete (win as { __TAURI__?: TauriBridge | null }).__TAURI__;
      } else {
        win.__TAURI__ = previous;
      }
    };
  },
};
