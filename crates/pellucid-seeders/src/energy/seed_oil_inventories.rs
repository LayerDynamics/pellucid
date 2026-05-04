//! seed_oil_inventories — FAST-tier snapshot of weekly US
//! crude petroleum stocks via EIA v2.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::energy::EnergySeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "eia:petroleum-stocks:latest:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "oil-inventories-eia-v2-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "energy-oil-inventories";

/// EIA dataset path for weekly US ending stocks of crude oil.
pub const DEFAULT_DATASET_PATH: &str = "petroleum/stoc/wstk";

/// Default number of weeks to keep in the snapshot.
pub const DEFAULT_LOOKBACK_WEEKS: u32 = 26;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct OilInventoriesConfig {
    /// EIA dataset path.
    pub dataset_path: String,
    /// Lookback window — last N weeks.
    pub lookback_weeks: u32,
}

impl Default for OilInventoriesConfig {
    fn default() -> Self {
        Self {
            dataset_path: DEFAULT_DATASET_PATH.to_string(),
            lookback_weeks: DEFAULT_LOOKBACK_WEEKS,
        }
    }
}

/// One per-week row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InventoryRow {
    /// `YYYY-MM-DD` period end.
    pub period: String,
    /// Stock value (units in `units`).
    pub value: f64,
    /// EIA-reported units (e.g. `"Thousand Barrels"`).
    pub units: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OilInventoriesSnapshot {
    /// Newest-first weekly readings.
    pub rows: Vec<InventoryRow>,
    /// Most-recent reading (the headline).
    pub latest: InventoryRow,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled EIA row — mirrors `pellucid_streams::EiaRow`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedEiaRow {
    /// Period.
    pub period: String,
    /// Value.
    pub value: f64,
    /// Units.
    pub units: String,
}

/// DI trait — wraps the production
/// `pellucid_streams::EiaClient::fetch` configured with
/// `dataset_path` + `frequency=weekly` + `length=lookback_weeks`.
#[async_trait]
pub trait EiaSeriesFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch up to `length` rows from `dataset_path` at the
    /// given frequency.
    async fn fetch_series(
        &self,
        dataset_path: &str,
        frequency: &str,
        length: u32,
    ) -> Result<Vec<FetchedEiaRow>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`EnergySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn EiaSeriesFetcher,
    config: &OilInventoriesConfig,
) -> Result<PublishOutcome, EnergySeederError> {
    let fetched = fetcher
        .fetch_series(&config.dataset_path, "weekly", config.lookback_weeks)
        .await
        .map_err(|e| EnergySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EnergySeederError::EmptyUpstream);
    }
    let rows: Vec<InventoryRow> = fetched
        .into_iter()
        .map(|r| InventoryRow {
            period: r.period,
            value: r.value,
            units: r.units,
        })
        .collect();
    let latest = rows
        .first()
        .cloned()
        .ok_or(EnergySeederError::EmptyUpstream)?;

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = OilInventoriesSnapshot {
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

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
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

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl EiaSeriesFetcher for FailingFetcher {
        async fn fetch_series(
            &self,
            _dataset_path: &str,
            _frequency: &str,
            _length: u32,
        ) -> Result<Vec<FetchedEiaRow>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn row(period: &str, value: f64) -> FetchedEiaRow {
        FetchedEiaRow {
            period: period.into(),
            value,
            units: "Thousand Barrels".into(),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "eia:petroleum-stocks:latest:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_latest() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                row("2026-04-25", 457_321.0),
                row("2026-04-18", 456_900.0),
                row("2026-04-11", 455_500.0),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &OilInventoriesConfig::default())
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
        assert_eq!(
            parsed.pointer("/data/latest/period").unwrap().as_str().unwrap(),
            "2026-04-25"
        );
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &OilInventoriesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, EnergySeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &OilInventoriesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, EnergySeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![row("2026-04-25", 457_321.0)],
        };
        let _ = run_cycle(&pool, &fetcher, &OilInventoriesConfig::default())
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
