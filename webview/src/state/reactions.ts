/**
 * Cross-store reactions. Wires variant changes to map-layer reset +
 * panel-disable, mirroring the original WorldMonitor `App.ts:424-449`
 * behaviour. T2.8 will extend with the per-variant migration-key
 * record-keeping.
 *
 * Reactions are *not* installed automatically at module import; the
 * 8-phase boot machine (T1.11) calls `installVariantReactions()` once
 * during P1 so unit tests of individual stores stay isolated.
 */

import { useMapStore } from "./useMapStore";
import { useVariantStore, type Variant } from "./useVariantStore";

/** Returns an unsubscribe function to detach the reactions during teardown. */
export function installVariantReactions(): () => void {
  const unsub = useVariantStore.subscribe(
    (s) => s.variant,
    (current: Variant, previous: Variant) => {
      if (current === previous) return;
      // Map-layer reset on variant flip — every variant defines its
      // own default layer set; the per-variant config modules (T2.8)
      // expose `defaultMapLayers(variant): string[]`.
      useMapStore.getState().resetLayers();
      // Mark switching as complete now that the cross-store side
      // effects fired.
      useVariantStore.setState({ switching: false });
    },
  );
  return unsub;
}
