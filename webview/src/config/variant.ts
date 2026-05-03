/**
 * Variant detection chain (OP-13).
 *
 * Port of the original WorldMonitor `src/config/variant.ts:1-32`:
 *
 * ```ts
 * function detectVariant(): Variant {
 *   const fromBuild = (import.meta.env.VITE_VARIANT as Variant) ?? null;
 *   const fromHost  = matchHostnamePrefix(window.location.hostname);
 *   const fromStore = isDesktop() ? store.get('variant') : localStorage.getItem('variant');
 *   return fromBuild ?? fromHost ?? (fromStore as Variant) ?? 'base';
 * }
 * ```
 *
 * Resolution order:
 *   1. Build-time `VITE_VARIANT` (set by the variant build
 *      matrix — `build:tech` etc.). Wins because it's
 *      compiled-in.
 *   2. Hostname prefix (`tech.worldmonitor.app` →
 *      `tech`). Lets the same web build serve every variant.
 *   3. Persisted store value (Tauri `store` plugin on desktop,
 *      `localStorage` on web). Survives reloads.
 *   4. `base` fallback.
 */

import { ALL_VARIANTS, type Variant } from "../state/useVariantStore";
import { VARIANT_CONFIGS } from "./variants";

/** Sources the detector consults, in priority order. */
export interface DetectionSources {
  /** `import.meta.env.VITE_VARIANT` value. `null` when unset. */
  fromBuild: string | null;
  /** Browser/desktop hostname (e.g. `"tech.worldmonitor.app"`). */
  fromHostname: string | null;
  /** Persisted user choice (Tauri `store` or `localStorage`). */
  fromStore: string | null;
}

/** A variant id that may have come from any source — typed
 *  loosely because the input strings are user/env-controlled. */
export type Candidate = string | null;

/**
 * Type-narrowing predicate: `true` iff `value` is one of the 5
 * known variant ids.
 */
export function isVariant(value: unknown): value is Variant {
  return (
    typeof value === "string" && (ALL_VARIANTS as readonly string[]).includes(value)
  );
}

/**
 * Resolve a hostname to a variant id by matching against the
 * registered `hostnamePrefix` of each variant. Returns `null`
 * when no variant claims the hostname (the bare domain → `base`
 * case is handled by the caller's fallback chain, not here).
 */
export function matchHostnamePrefix(hostname: string | null): Variant | null {
  if (!hostname) return null;
  const lower = hostname.toLowerCase();
  for (const cfg of Object.values(VARIANT_CONFIGS)) {
    if (cfg.hostnamePrefix && lower.startsWith(cfg.hostnamePrefix)) {
      return cfg.id;
    }
  }
  return null;
}

/**
 * Coerce an arbitrary string into a `Variant` if it matches one
 * of the 5 known ids; otherwise `null`.
 */
export function coerceVariant(candidate: Candidate): Variant | null {
  return isVariant(candidate) ? candidate : null;
}

/**
 * Run the full detection chain against an explicit
 * [`DetectionSources`] snapshot. Pure function — easy to drive
 * from unit tests without touching `import.meta` / `window` /
 * `localStorage`.
 */
export function detectVariantFrom(sources: DetectionSources): Variant {
  return (
    coerceVariant(sources.fromBuild) ??
    matchHostnamePrefix(sources.fromHostname) ??
    coerceVariant(sources.fromStore) ??
    "base"
  );
}

/**
 * Read the live process / runtime values + run the chain. Used
 * by the boot machine; tests should call [`detectVariantFrom`]
 * with a hand-constructed snapshot instead of mocking globals.
 */
export function detectVariant(): Variant {
  return detectVariantFrom({
    fromBuild: readBuildVariant(),
    fromHostname: readHostname(),
    fromStore: readStoredVariant(),
  });
}

function readBuildVariant(): string | null {
  // `import.meta.env.VITE_VARIANT` is set at compile time by Vite
  // when the build is run with the variant-specific scripts.
  // Available in both the web bundle and the Tauri webview.
  if (typeof import.meta !== "undefined" && (import.meta as ImportMeta).env) {
    const env = (import.meta as ImportMeta).env as Record<string, string | undefined>;
    return env.VITE_VARIANT ?? null;
  }
  return null;
}

function readHostname(): string | null {
  if (typeof window === "undefined") return null;
  // `window.location.hostname` may be `""` (file:// in some
  // sandboxes); coerce empty to null.
  const host = window.location?.hostname;
  return host && host.length > 0 ? host : null;
}

function readStoredVariant(): string | null {
  if (typeof window === "undefined") return null;
  // The Tauri `store` plugin shadow lives at this key when
  // desktop. On web we read directly from `localStorage`. The
  // store plugin's read is async; the synchronous detector
  // accepts the localStorage value and the boot machine may
  // re-resolve via `setVariant` once the async store loads.
  try {
    return window.localStorage?.getItem("pellucid:variant") ?? null;
  } catch {
    // localStorage can throw when disabled (private mode,
    // sandboxed iframe). The detector falls through to `base`.
    return null;
  }
}
