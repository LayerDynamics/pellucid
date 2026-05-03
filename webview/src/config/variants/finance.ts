import type { VariantConfig } from "./types";

/**
 * Finance variant — markets, economic indicators, sanctions.
 */
export const financeConfig: VariantConfig = {
  id: "finance",
  displayName: "Pellucid Finance",
  hostnamePrefix: "finance.",
  defaultMapLayers: ["sanctions-overlay", "trade-flows"],
  allowedPanels: [
    "market/indices-snapshot",
    "market/fx-snapshot",
    "market/commodities-snapshot",
    "market/stocks-bootstrap",
    "market/analyst-summary",
    "economic/indicator",
    "economic/fred-latest",
    "consumer-prices/cpi",
    "trade/tariff-alerts",
    "trade/tariff-catalog",
    "sanctions/entity-list",
    "sanctions/recent-additions",
    "supply-chain/stress-index",
    "supply-chain/port-congestion",
    "news/breaking",
    "intelligence/correlation-graph",
  ],
  defaultPanelOrder: [
    "market/indices-snapshot",
    "market/fx-snapshot",
    "economic/fred-latest",
    "sanctions/recent-additions",
    "news/breaking",
  ],
  migrationKey: "FINANCE_VARIANT_MIGRATION_v1",
};
