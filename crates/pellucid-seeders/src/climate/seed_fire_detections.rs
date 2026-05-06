//! seed_fire_detections — FAST-tier snapshot of the worst
//! VIIRS active-fire detections in the last 24 hours.
//!
//! NASA FIRMS publishes 5-50k detections per day; the panel
//! only renders the worst few hundred. The seeder fetches the
//! global CSV, ranks by Fire Radiative Power (FRP, MW), and
//! keeps the top-`limit` rows.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::climate::ClimateSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "wildfire:active-perimeters:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "fire-detections-firms-viirs-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "wildfire";

/// Default top-N cap on detections kept in the snapshot.
pub const DEFAULT_TOP_LIMIT: usize = 500;

/// Default minimum FRP (MW) — discards weak signals.
pub const DEFAULT_MIN_FRP: f64 = 1.0;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct FireDetectionsConfig {
    /// Top-N cap.
    pub top_limit: usize,
    /// Minimum FRP threshold.
    pub min_frp: f64,
}

impl Default for FireDetectionsConfig {
    fn default() -> Self {
        Self {
            top_limit: DEFAULT_TOP_LIMIT,
            min_frp: DEFAULT_MIN_FRP,
        }
    }
}

/// One detection row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FireDetectionRow {
    /// WGS84 latitude.
    pub latitude: f64,
    /// WGS84 longitude.
    pub longitude: f64,
    /// Fire Radiative Power (megawatts).
    pub frp: f64,
    /// VIIRS Brightness Temperature I-4 channel (Kelvin).
    pub bright_ti4: f64,
    /// `YYYY-MM-DD` UTC date.
    pub acq_date: String,
    /// `HHMM` UTC time.
    pub acq_time: String,
    /// Confidence label (`l` / `n` / `h`).
    pub confidence: String,
    /// Day/night flag (`D` / `N`).
    pub daynight: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FireDetectionsSnapshot {
    /// Top-N detections by FRP (descending).
    pub rows: Vec<FireDetectionRow>,
    /// Total raw count returned by the upstream (before
    /// `top_limit` truncation + `min_frp` filter).
    pub total_detections: usize,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled detection — mirrors `pellucid_streams::FireDetection`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedDetection {
    /// WGS84 latitude.
    pub latitude: f64,
    /// WGS84 longitude.
    pub longitude: f64,
    /// FRP in megawatts.
    pub frp: f64,
    /// VIIRS BTI-4 channel.
    pub bright_ti4: f64,
    /// UTC date.
    pub acq_date: String,
    /// UTC time.
    pub acq_time: String,
    /// Confidence label.
    pub confidence: String,
    /// Day/night flag.
    pub daynight: String,
}

/// DI trait — wraps `pellucid_streams::NasaFirmsClient::fetch_global_24h`.
#[async_trait]
pub trait FireDetectionsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the global VIIRS 24-hour active-fire CSV.
    async fn fetch_global_24h(
        &self,
    ) -> Result<Vec<FetchedDetection>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`ClimateSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn FireDetectionsFetcher,
    config: &FireDetectionsConfig,
) -> Result<PublishOutcome, ClimateSeederError> {
    let fetched = fetcher
        .fetch_global_24h()
        .await
        .map_err(|e| ClimateSeederError::Upstream(e.to_string()))?;
    let total_detections = fetched.len();
    let mut rows: Vec<FireDetectionRow> = fetched
        .into_iter()
        .filter(|d| d.frp >= config.min_frp)
        .map(|d| FireDetectionRow {
            latitude: d.latitude,
            longitude: d.longitude,
            frp: d.frp,
            bright_ti4: d.bright_ti4,
            acq_date: d.acq_date,
            acq_time: d.acq_time,
            confidence: d.confidence,
            daynight: d.daynight,
        })
        .collect();
    if rows.is_empty() {
        return Err(ClimateSeederError::EmptyUpstream);
    }
    rows.sort_by(|a, b| {
        b.frp
            .partial_cmp(&a.frp)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(config.top_limit);

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = FireDetectionsSnapshot {
        rows,
        total_detections,
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
    let outcome = atomic_publish(pool, "wildfire", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedDetection>,
    }

    #[async_trait]
    impl FireDetectionsFetcher for StaticFetcher {
        async fn fetch_global_24h(
            &self,
        ) -> Result<Vec<FetchedDetection>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl FireDetectionsFetcher for FailingFetcher {
        async fn fetch_global_24h(
            &self,
        ) -> Result<Vec<FetchedDetection>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn det(frp: f64, lat: f64, lon: f64) -> FetchedDetection {
        FetchedDetection {
            latitude: lat,
            longitude: lon,
            frp,
            bright_ti4: 320.0,
            acq_date: "2026-05-04".into(),
            acq_time: "0123".into(),
            confidence: "h".into(),
            daynight: "D".into(),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "wildfire:active-perimeters:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_top_n_sorted_by_frp_desc() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                det(5.0, 40.0, -118.0),
                det(50.0, 35.0, -120.0), // top
                det(0.5, 30.0, -115.0),  // dropped (below min_frp)
                det(20.0, 41.0, -117.0),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &FireDetectionsConfig::default())
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
        assert!((rows[0].get("frp").unwrap().as_f64().unwrap() - 50.0).abs() < 1e-9);
        assert!((rows[1].get("frp").unwrap().as_f64().unwrap() - 20.0).abs() < 1e-9);
        assert!((rows[2].get("frp").unwrap().as_f64().unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(
            parsed.pointer("/data/total_detections").unwrap().as_u64(),
            Some(4)
        );
    }

    #[tokio::test]
    async fn run_cycle_truncates_to_top_limit() {
        let pool = open_in_memory().await.unwrap();
        let mut rows: Vec<FetchedDetection> = (0..1000)
            .map(|i| det(f64::from(i) + 1.0, 40.0, -118.0))
            .collect();
        rows.reverse(); // give the seeder unsorted input
        let fetcher = StaticFetcher { rows };
        let cfg = FireDetectionsConfig {
            top_limit: 25,
            min_frp: 1.0,
        };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 25);
        // Top entry must be FRP=1000.0
        assert!((rows[0].get("frp").unwrap().as_f64().unwrap() - 1000.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn run_cycle_empty_after_filter_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![det(0.5, 0.0, 0.0), det(0.1, 0.0, 0.0)],
        };
        let err = run_cycle(&pool, &fetcher, &FireDetectionsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ClimateSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &FireDetectionsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ClimateSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![det(50.0, 40.0, -118.0)],
        };
        let _ = run_cycle(&pool, &fetcher, &FireDetectionsConfig::default())
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
