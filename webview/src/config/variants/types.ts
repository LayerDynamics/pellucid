import type { Variant } from "../../state/useVariantStore";

/**
 * Per-variant configuration. Drives the variant detection chain
 * (`hostnamePrefix`) and the cross-store reactions (defaults,
 * allow-list, migration recording).
 */
export interface VariantConfig {
  /** Variant id matching `useVariantStore`. */
  id: Variant;
  /** Human-readable name shown in the title bar / about page. */
  displayName: string;
  /**
   * Hostname prefix the detector matches against. `null` for
   * `base` (the bare domain).
   */
  hostnamePrefix: string | null;
  /**
   * Map layer ids to seed when the variant is selected. The
   * cross-store reaction calls `useMapStore.setLayers(...)` with
   * this list immediately after `resetLayers()`.
   */
  defaultMapLayers: string[];
  /**
   * Panel ids permitted under this variant. `["*"]` (with `*` as
   * a single entry) means "every panel" — only `base` uses this
   * sentinel.
   */
  allowedPanels: string[];
  /**
   * Panel ids in the order they should appear when the variant
   * has just been selected (and no user reordering exists).
   */
  defaultPanelOrder: string[];
  /**
   * Migration key recorded in `localStorage` /
   * `usePanelStore.migrations` once the variant's defaults have
   * been applied. Mirrors `PANEL_KEY_RENAMES_MIGRATION_KEY`,
   * `UNIFIED_MIGRATION_KEY`, `HAPPY_PANEL_FIX_KEY` from the
   * original WorldMonitor codebase.
   */
  migrationKey: string;
}
