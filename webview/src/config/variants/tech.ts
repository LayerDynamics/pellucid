import type { VariantConfig } from "./types";

/**
 * Tech variant — cyber, research, infrastructure focus. Hostname
 * prefix `tech.worldmonitor.app` (per SPEC-001 §16.1) routes
 * here at build time.
 */
export const techConfig: VariantConfig = {
  id: "tech",
  displayName: "Pellucid Tech",
  hostnamePrefix: "tech.",
  defaultMapLayers: ["cyber-incidents", "research-arxiv-pulses"],
  allowedPanels: [
    "cyber/incident-feed",
    "cyber/cve-detail",
    "cyber/active-campaigns",
    "infrastructure/grid-stress",
    "infrastructure/pipeline-flows",
    "research/arxiv-feed",
    "research/paper-catalog",
    "intelligence/correlation-graph",
    "intelligence/hot-stories",
    "news/breaking",
    "news/signals",
    "discord/active-channels",
  ],
  defaultPanelOrder: [
    "cyber/incident-feed",
    "research/arxiv-feed",
    "intelligence/correlation-graph",
    "news/breaking",
  ],
  migrationKey: "TECH_VARIANT_MIGRATION_v1",
};
