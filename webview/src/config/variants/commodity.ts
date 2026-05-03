import type { VariantConfig } from "./types";

/**
 * Commodity variant — energy, supply chain, maritime, climate.
 */
export const commodityConfig: VariantConfig = {
  id: "commodity",
  displayName: "Pellucid Commodity",
  hostnamePrefix: "commodity.",
  defaultMapLayers: [
    "ais-vessels",
    "supply-chain-routes",
    "eia-petroleum-stocks",
  ],
  allowedPanels: [
    "eia/petroleum-stocks",
    "eia/nat-gas-spot",
    "supply-chain/stress-index",
    "supply-chain/port-congestion",
    "supply-chain/route-topology",
    "maritime/ais-snapshot",
    "maritime/chokepoint-status",
    "maritime/active-incidents",
    "climate/anomaly-grid",
    "climate/station-record",
    "forecast/extended",
    "trade/tariff-alerts",
    "market/commodities-snapshot",
    "news/breaking",
    "infrastructure/pipeline-flows",
  ],
  defaultPanelOrder: [
    "eia/petroleum-stocks",
    "supply-chain/stress-index",
    "maritime/chokepoint-status",
    "market/commodities-snapshot",
    "news/breaking",
  ],
  migrationKey: "COMMODITY_VARIANT_MIGRATION_v1",
};
