import type { VariantConfig } from "./types";

/**
 * Happy variant — positive-events / human-interest skin. The
 * inverted-mood-monitor product — same data plumbing, opposite
 * editorial selection. Per LoreDeepCodeReview.md §1.6 the
 * `HAPPY_PANEL_FIX_KEY` migration is recorded here so the
 * cross-store reactions clear panels that bled in from other
 * variants.
 */
export const happyConfig: VariantConfig = {
  id: "happy",
  displayName: "Pellucid Happy",
  hostnamePrefix: "happy.",
  defaultMapLayers: ["positive-events"],
  allowedPanels: [
    "positive-events/feed",
    "positive-events/archive",
    "giving/donor-catalog",
    "research/arxiv-feed",
    "skills/active",
    "news/breaking",
  ],
  defaultPanelOrder: [
    "positive-events/feed",
    "giving/donor-catalog",
    "skills/active",
  ],
  migrationKey: "HAPPY_PANEL_FIX_KEY",
};
