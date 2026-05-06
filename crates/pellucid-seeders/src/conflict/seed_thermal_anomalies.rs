//! seed_thermal_anomalies — FAST-tier MODIS / VIIRS thermal-
//! anomaly snapshot, escalation-risk filter. Production adapters
//! wire to NASA FIRMS thermal-anomaly CSV (same upstream as
//! wildfires) but with a tighter geo filter on conflict zones.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::conflict::ConflictSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier.
pub const CACHE_KEY: &str = "thermal:anomaly-feed:current:v1";

/// 15 m TTL — anomaly stream churns frequently.
pub const TTL: Duration = Duration::from_secs(15 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "thermal-anomalies-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "thermal";

/// One thermal-anomaly row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThermalRow {
    /// Anomaly id.
    pub id: String,
    /// Latitude.
    pub lat: f64,
    /// Longitude.
    pub lon: f64,
    /// Brightness in Kelvin.
    pub brightness_k: f64,
    /// Confidence 0..=100 (higher = more reliable).
    pub confidence: u32,
    /// Conflict zone tag (e.g. `Ukraine`, `Gaza`, `Sudan`).
    pub zone: String,
    /// ISO-8601 acquisition timestamp.
    pub acquired_at: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThermalSnapshot {
    /// Rows sorted by descending brightness.
    pub rows: Vec<ThermalRow>,
    /// Total anomaly count.
    pub total: usize,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedThermalRow {
    /// Anomaly id.
    pub id: String,
    /// Latitude.
    pub lat: f64,
    /// Longitude.
    pub lon: f64,
    /// Brightness K.
    pub brightness_k: f64,
    /// Confidence 0..=100.
    pub confidence: u32,
    /// Conflict zone tag.
    pub zone: String,
    /// Acquired-at.
    pub acquired_at: String,
}

/// DI trait.
#[async_trait]
pub trait ThermalAnomaliesFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch thermal anomalies in conflict zones.
    async fn fetch_anomalies(
        &self,
    ) -> Result<Vec<FetchedThermalRow>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn ThermalAnomaliesFetcher,
) -> Result<PublishOutcome, ConflictSeederError> {
    let fetched = fetcher
        .fetch_anomalies()
        .await
        .map_err(|e| ConflictSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(ConflictSeederError::EmptyUpstream);
    }
    let mut rows: Vec<ThermalRow> = fetched
        .into_iter()
        .map(|r| ThermalRow {
            id: r.id,
            lat: r.lat,
            lon: r.lon,
            brightness_k: r.brightness_k,
            confidence: r.confidence,
            zone: r.zone,
            acquired_at: r.acquired_at,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.brightness_k
            .partial_cmp(&a.brightness_k)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total = rows.len();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = ThermalSnapshot {
        rows,
        total,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(900_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "thermal", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedThermalRow>,
    }

    #[async_trait]
    impl ThermalAnomaliesFetcher for StaticFetcher {
        async fn fetch_anomalies(
            &self,
        ) -> Result<Vec<FetchedThermalRow>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    fn anom(id: &str, k: f64) -> FetchedThermalRow {
        FetchedThermalRow {
            id: id.into(),
            lat: 50.45,
            lon: 30.52,
            brightness_k: k,
            confidence: 80,
            zone: "Ukraine".into(),
            acquired_at: "2026-04-29T12:00:00Z".into(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "thermal:anomaly-feed:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_by_brightness() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![anom("a", 320.0), anom("b", 410.0), anom("c", 360.0)],
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let ids: Vec<&str> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("id").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, ConflictSeederError::EmptyUpstream));
    }
}
