//! seed_fuel_prices — FAST-tier snapshot of US weekly retail
//! gasoline prices via EIA v2 (`petroleum/pri/gnd`).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::energy::seed_oil_inventories::{EiaSeriesFetcher, FetchedEiaRow};
use crate::energy::EnergySeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — NEW FAST tier slot added by T3.8 energy domain.
pub const CACHE_KEY: &str = "energy:fuel-prices:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "fuel-prices-eia-v2-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "energy-fuel-prices";

/// EIA dataset path for weekly retail gasoline prices.
pub const DEFAULT_DATASET_PATH: &str = "petroleum/pri/gnd";

/// Default lookback window — last N weeks.
pub const DEFAULT_LOOKBACK_WEEKS: u32 = 26;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct FuelPricesConfig {
    /// EIA dataset path.
    pub dataset_path: String,
    /// Lookback window.
    pub lookback_weeks: u32,
}

impl Default for FuelPricesConfig {
    fn default() -> Self {
        Self {
            dataset_path: DEFAULT_DATASET_PATH.to_string(),
            lookback_weeks: DEFAULT_LOOKBACK_WEEKS,
        }
    }
}

/// One per-week price row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PriceRow {
    /// `YYYY-MM-DD` period end.
    pub period: String,
    /// Price (units in `units`).
    pub value: f64,
    /// Units (e.g. `"$/Gallon"`).
    pub units: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FuelPricesSnapshot {
    /// Newest-first weekly readings.
    pub rows: Vec<PriceRow>,
    /// Most-recent reading.
    pub latest: PriceRow,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Run one cycle.
///
/// # Errors
/// See [`EnergySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn EiaSeriesFetcher,
    config: &FuelPricesConfig,
) -> Result<PublishOutcome, EnergySeederError> {
    let fetched = fetcher
        .fetch_series(&config.dataset_path, "weekly", config.lookback_weeks)
        .await
        .map_err(|e| EnergySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EnergySeederError::EmptyUpstream);
    }
    let rows: Vec<PriceRow> = fetched.into_iter().map(map_row).collect();
    let latest = rows.first().cloned().ok_or(EnergySeederError::EmptyUpstream)?;

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = FuelPricesSnapshot {
        rows,
        latest,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(60_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome =
        atomic_publish(pool, "energy", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn map_row(r: FetchedEiaRow) -> PriceRow {
    PriceRow {
        period: r.period,
        value: r.value,
        units: r.units,
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
        rows: Vec<FetchedEiaRow>,
    }

    #[async_trait]
    impl EiaSeriesFetcher for StaticFetcher {
        async fn fetch_series(
            &self,
            _dataset_path: &str,
            _frequency: &str,
            _length: u32,
        ) -> Result<Vec<FetchedEiaRow>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    fn row(period: &str, value: f64) -> FetchedEiaRow {
        FetchedEiaRow {
            period: period.into(),
            value,
            units: "$/Gallon".into(),
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "energy:fuel-prices:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![row("2026-04-25", 3.45), row("2026-04-18", 3.42)],
        };
        let outcome = run_cycle(&pool, &fetcher, &FuelPricesConfig::default())
            .await
            .unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        assert!(
            (parsed.pointer("/data/latest/value").unwrap().as_f64().unwrap() - 3.45)
                .abs()
                < 1e-9
        );
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &FuelPricesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, EnergySeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![row("2026-04-25", 3.45)],
        };
        let _ = run_cycle(&pool, &fetcher, &FuelPricesConfig::default())
            .await
            .unwrap();
        let meta: (String, String) = sqlx::query_as(
            "SELECT source_version, cascade_group FROM seed_meta WHERE cache_key = ?",
        )
        .bind(CACHE_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(meta.0, SOURCE_VERSION);
        assert_eq!(meta.1, CASCADE_GROUP);
    }
}
