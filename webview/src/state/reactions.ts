/**
 * Cross-store reactions. Wires variant changes to the full
 * cross-store side-effect set documented in SPEC-001 §16.2 +
 * the original WorldMonitor `App.ts:424-449`:
 *
 *   1. Reset map layers (`useMapStore.resetLayers`).
 *   2. Apply the variant's default map layer set.
 *   3. Hide every panel NOT in the target variant's allow-list.
 *   4. Show panels in the allow-list (un-hide if they had been
 *      hidden under a previous variant).
 *   5. Record the variant's `migrationKey` in
 *      `localStorage` so re-renders + reloads don't re-run the
 *      reset (idempotency mirroring the legacy
 *      `PANEL_KEY_RENAMES_MIGRATION_KEY` /
 *      `UNIFIED_MIGRATION_KEY` / `HAPPY_PANEL_FIX_KEY` keys).
 *   6. Push the new variant to the Tauri host (no-op on web).
 *   7. Mark switching = false.
 *
 * Reactions are *not* installed automatically at module import;
 * the 8-phase boot machine (T1.11) calls
 * `installVariantReactions()` once during P1 so unit tests of
 * individual stores stay isolated.
 */

import { configFor } from "../config/variants";
import { pushVariantToHost } from "../services/runtime";
import { useMapStore } from "./useMapStore";
import { usePanelStore } from "./usePanelStore";
import { useVariantStore, type Variant } from "./useVariantStore";

/** localStorage key prefix the migration record uses. The full
 *  key is `pellucid:variant-migration:<variant-id>`. */
export const MIGRATION_STORAGE_PREFIX = "pellucid:variant-migration:";

/** Returns an unsubscribe function to detach the reactions during
 *  teardown. Tests use this to avoid leaking subscriptions
 *  across cases. */
export function installVariantReactions(): () => void {
  const unsub = useVariantStore.subscribe(
    (s) => s.variant,
    (current: Variant, previous: Variant) => {
      if (current === previous) return;
      applyVariantReactions(current);
    },
  );
  return unsub;
}

/**
 * Apply every cross-store reaction for `variant`. Exposed so the
 * boot machine can invoke it at the end of P1 even without a
 * variant change (initial settle), and tests can drive it
 * deterministically without going through the subscribe path.
 */
export function applyVariantReactions(variant: Variant): void {
  const cfg = configFor(variant);

  // 1 + 2: reset layers + seed defaults.
  useMapStore.getState().resetLayers();
  useMapStore.getState().setLayers([...cfg.defaultMapLayers]);

  // 3 + 4: enforce panel allow-list.
  const panelStore = usePanelStore.getState();
  const registered = panelStore.registered();
  const isWildcard =
    cfg.allowedPanels.length === 1 && cfg.allowedPanels[0] === "*";
  const allowed = new Set(cfg.allowedPanels);
  for (const panelId of registered) {
    const permitted = isWildcard || allowed.has(panelId);
    if (permitted) {
      panelStore.show(panelId);
    } else {
      panelStore.hide(panelId);
    }
  }

  // 5: record the migration key.
  recordMigration(variant, cfg.migrationKey);

  // 6: notify the Tauri host (no-op on web).
  void pushVariantToHost(variant);

  // 7: clear the switching flag.
  useVariantStore.setState({ switching: false });
}

function recordMigration(variant: Variant, migrationKey: string): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    window.localStorage.setItem(
      `${MIGRATION_STORAGE_PREFIX}${variant}`,
      migrationKey,
    );
  } catch {
    // localStorage may throw in private mode / sandboxed iframes.
    // The reaction is best-effort — losing the migration record
    // just means the next switch re-runs, which is idempotent.
  }
}

/** Diagnostic: read the recorded migration key for a variant.
 *  Returns `null` when the key was never written (dev mode,
 *  fresh install, localStorage unavailable). */
export function readRecordedMigration(variant: Variant): string | null {
  if (typeof window === "undefined" || !window.localStorage) return null;
  try {
    return window.localStorage.getItem(`${MIGRATION_STORAGE_PREFIX}${variant}`);
  } catch {
    return null;
  }
}
