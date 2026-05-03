import type { VariantConfig } from "./types";

/**
 * Base variant — the canonical Pellucid skin. Every panel is
 * allow-listed; default map layers cover the cross-domain
 * intelligence overlay.
 */
export const baseConfig: VariantConfig = {
  id: "base",
  displayName: "Pellucid",
  hostnamePrefix: null,
  defaultMapLayers: ["news-pulses", "ais-vessels", "opensky-aircraft"],
  allowedPanels: ["*"],
  defaultPanelOrder: [
    "aviation/flight-status",
    "news/breaking",
    "maritime/ais-snapshot",
    "intelligence/correlation-graph",
  ],
  migrationKey: "BASE_VARIANT_MIGRATION_v1",
};
