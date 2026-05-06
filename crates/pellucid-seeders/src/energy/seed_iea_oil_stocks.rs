//! seed_iea_oil_stocks — SLOW-tier snapshot of OECD oil-stocks
//! data.
//!
//! IEA's Monthly Oil Statistics service is paywalled. JODI's
//! `world_oil.csv` publishes the same supply-side data
//! (`STOCKCH` flow breakdown for stock changes;
//! `CLOSTLV` for closing levels) under a free public licence.
//! The seeder uses JODI as the canonical free proxy and
//! documents this clearly.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::energy::seed_jodi::{FetchedJodiRow, JodiFetcher};
use crate::energy::EnergySeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — NEW SLOW tier slot added by T3.8 energy domain.
pub const CACHE_KEY: &str = "energy:iea-oil-stocks:monthly:v1";

/// SLOW-tier TTL — 12 hours. Stocks data refreshes at most
/// monthly.
pub const TTL: Duration = Duration::from_secs(12 * 60 * 60);

/// Source-version stamp. Documents the JODI proxy choice.
pub const SOURCE_VERSION: &str = "iea-oil-stocks-via-jodi-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "energy-iea-oil-stocks";

/// JODI flow breakdown for closing stock levels.
pub const DEFAULT_FLOW_BREAKDOWN: &str = "CLOSTLV";

/// Default OECD basket — major reporting countries that have
/// IEA-aligned coverage.
pub const DEFAULT_COUNTRIES: &[&str] = &[
    "USA", "JPN", "DEU", "FRA", "GBR", "ITA", "ESP", "CAN", "KOR", "NLD",
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct IeaOilStocksConfig {
    /// OECD-aligned country codes to keep.
    pub countries: Vec<String>,
    /// JODI flow breakdown filter.
    pub flow_breakdown: String,
}

impl Default for IeaOilStocksConfig {
    fn default() -> Self {
        Self {
            countries: DEFAULT_COUNTRIES.iter().map(|s| (*s).to_string()).collect(),
            flow_breakdown: DEFAULT_FLOW_BREAKDOWN.to_string(),
        }
    }
}

/// One per-country/period stock-level reading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StockLevelRow {
    /// ISO 3-letter country code.
    pub country: String,
    /// `YYYY-MM` reporting period.
    pub time_period: String,
    /// Closing stock level.
    pub obs_value: f64,
    /// Units (`KBL` for thousand barrels).
    pub unit_measure: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IeaOilStocksSnapshot {
    /// Per-country stock rows for the most recent period.
    pub rows: Vec<StockLevelRow>,
    /// Source-data note documenting the JODI proxy.
    pub source_note: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Run one cycle.
///
/// # Errors
/// See [`EnergySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn JodiFetcher,
    config: &IeaOilStocksConfig,
) -> Result<PublishOutcome, EnergySeederError> {
    let fetched = fetcher
        .fetch_world(true, None, Some(&config.flow_breakdown))
        .await
        .map_err(|e| EnergySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EnergySeederError::EmptyUpstream);
    }
    let latest_period = fetched
        .iter()
        .map(|r| r.time_period.clone())
        .max()
        .ok_or(EnergySeederError::EmptyUpstream)?;
    let countries: std::collections::HashSet<String> = config.countries.iter().cloned().collect();
    let rows: Vec<StockLevelRow> = fetched
        .into_iter()
        .filter(|r| r.time_period == latest_period && countries.contains(&r.country))
        .map(map_row)
        .collect();
    if rows.is_empty() {
        return Err(EnergySeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = IeaOilStocksSnapshot {
        rows,
        source_note: "Closing stock levels derived from JODI World Oil dataset (IEA Monthly Oil Statistics requires paid subscription)".into(),
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(43_200_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "energy", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn map_row(r: FetchedJodiRow) -> StockLevelRow {
    StockLevelRow {
        country: r.country,
        time_period: r.time_period,
        obs_value: r.obs_value,
        unit_measure: r.unit_measure,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedJodiRow>,
    }

    #[async_trait]
    impl JodiFetcher for StaticFetcher {
        async fn fetch_world(
            &self,
            _oil_dataset: bool,
            _country: Option<&str>,
            _flow_breakdown: Option<&str>,
        ) -> Result<Vec<FetchedJodiRow>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    fn row(country: &str, period: &str, value: f64) -> FetchedJodiRow {
        FetchedJodiRow {
            country: country.into(),
            energy_product: "CRUDEOIL".into(),
            flow_breakdown: "CLOSTLV".into(),
            unit_measure: "KBL".into(),
            time_period: period.into(),
            obs_value: value,
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "energy:iea-oil-stocks:monthly:v1");
    }

    #[tokio::test]
    async fn run_cycle_publishes_oecd_basket_with_source_note() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                row("USA", "2026-02", 700_000.0),
                row("USA", "2026-03", 705_000.0),
                row("JPN", "2026-03", 250_000.0),
                row("DEU", "2026-03", 175_000.0),
                row("ZWE", "2026-03", 5_000.0), // dropped
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &IeaOilStocksConfig::default())
            .await
            .unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 3);
        let note = parsed
            .pointer("/data/source_note")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(note.contains("JODI"));
        assert!(note.contains("paid subscription"));
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &IeaOilStocksConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, EnergySeederError::EmptyUpstream));
    }
}
