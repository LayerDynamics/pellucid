//! seed_spr_policies — SLOW-tier snapshot of US Strategic
//! Petroleum Reserve levels via EIA v2.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::energy::seed_oil_inventories::EiaSeriesFetcher;
use crate::energy::EnergySeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — NEW SLOW tier slot added by T3.8 energy domain.
pub const CACHE_KEY: &str = "energy:spr-status:current:v1";

/// SLOW-tier TTL — 1 hour. SPR levels move on weekly cadence
/// at most.
pub const TTL: Duration = Duration::from_secs(60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "spr-policies-eia-v2-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "energy-spr";

/// EIA dataset path for the petroleum supply-and-disposition
/// summary that includes SPR levels.
pub const DEFAULT_DATASET_PATH: &str = "petroleum/sum/snd";

/// Default lookback window — last N weeks.
pub const DEFAULT_LOOKBACK_WEEKS: u32 = 52;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct SprPoliciesConfig {
    /// EIA dataset path.
    pub dataset_path: String,
    /// Lookback window.
    pub lookback_weeks: u32,
}

impl Default for SprPoliciesConfig {
    fn default() -> Self {
        Self {
            dataset_path: DEFAULT_DATASET_PATH.to_string(),
            lookback_weeks: DEFAULT_LOOKBACK_WEEKS,
        }
    }
}

/// One per-week SPR row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SprRow {
    /// `YYYY-MM-DD` period end.
    pub period: String,
    /// SPR holdings (units in `units`).
    pub value: f64,
    /// Units (`"Thousand Barrels"`).
    pub units: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SprSnapshot {
    /// Newest-first weekly SPR readings.
    pub rows: Vec<SprRow>,
    /// Most-recent reading.
    pub latest: SprRow,
    /// Year-over-year delta vs the reading 52 weeks ago (when
    /// the lookback window has the data).
    pub yoy_delta: Option<f64>,
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
    config: &SprPoliciesConfig,
) -> Result<PublishOutcome, EnergySeederError> {
    let fetched = fetcher
        .fetch_series(&config.dataset_path, "weekly", config.lookback_weeks)
        .await
        .map_err(|e| EnergySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EnergySeederError::EmptyUpstream);
    }
    let rows: Vec<SprRow> = fetched
        .into_iter()
        .map(|r| SprRow {
            period: r.period,
            value: r.value,
            units: r.units,
        })
        .collect();
    let latest = rows
        .first()
        .cloned()
        .ok_or(EnergySeederError::EmptyUpstream)?;
    let yoy_delta = if rows.len() >= 52 {
        Some(latest.value - rows[51].value)
    } else {
        None
    };

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = SprSnapshot {
        rows,
        latest,
        yoy_delta,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(3_600_000),
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

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pellucid_db::open_in_memory;

    use crate::energy::seed_oil_inventories::FetchedEiaRow;

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
            units: "Thousand Barrels".into(),
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "energy:spr-status:current:v1");
    }

    #[test]
    fn ttl_is_slow_tier_one_hour() {
        assert_eq!(TTL, Duration::from_secs(60 * 60));
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_yoy_delta() {
        let pool = open_in_memory().await.unwrap();
        // Build 52 rows so YoY delta computes.
        let mut rows: Vec<FetchedEiaRow> = Vec::with_capacity(52);
        for i in 0..52 {
            rows.push(row(
                &format!("2026-w{i:02}"),
                360_000.0 + (51 - i) as f64 * 100.0,
            ));
        }
        // rows[0] = latest = 360_000 + 5100 = 365_100; rows[51] = 360_000.
        let fetcher = StaticFetcher { rows };
        let outcome = run_cycle(&pool, &fetcher, &SprPoliciesConfig::default())
            .await
            .unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let yoy = parsed.pointer("/data/yoy_delta").unwrap().as_f64().unwrap();
        assert!((yoy - 5100.0).abs() < 1e-3);
    }

    #[tokio::test]
    async fn run_cycle_short_window_yields_null_yoy_delta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![row("2026-04-25", 360_000.0), row("2026-04-18", 359_500.0)],
        };
        let _ = run_cycle(&pool, &fetcher, &SprPoliciesConfig::default())
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        assert!(parsed.pointer("/data/yoy_delta").unwrap().is_null());
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &SprPoliciesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, EnergySeederError::EmptyUpstream));
    }
}
