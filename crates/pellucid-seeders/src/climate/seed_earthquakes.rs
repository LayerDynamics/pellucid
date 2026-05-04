//! seed_earthquakes — FAST-tier snapshot of recent earthquakes.
//!
//! USGS publishes the last 24 hours of all earthquakes via
//! their `all_day` GeoJSON feed. The seeder takes the full
//! list, filters to magnitude >= `min_magnitude`, and keeps
//! everything (the panel renders all qualifying events on a
//! map; the `min_magnitude` cap keeps the row count bounded).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::climate::ClimateSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "seismology:recent-quakes:24h:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "earthquakes-usgs-allday-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "seismology";

/// Default minimum magnitude — drops sub-2.5 microquakes that
/// crowd the panel.
pub const DEFAULT_MIN_MAGNITUDE: f64 = 2.5;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct EarthquakesConfig {
    /// Minimum magnitude — events below this are dropped.
    pub min_magnitude: f64,
}

impl Default for EarthquakesConfig {
    fn default() -> Self {
        Self {
            min_magnitude: DEFAULT_MIN_MAGNITUDE,
        }
    }
}

/// One earthquake row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EarthquakeRow {
    /// Magnitude.
    pub magnitude: f64,
    /// Human-readable place description.
    pub place: String,
    /// Wall-clock ms when the event occurred.
    pub time_ms: i64,
    /// USGS event-page URL.
    pub url: String,
    /// `1` if a tsunami warning was issued, else `0`.
    pub tsunami: i64,
    /// PAGER alert level (`"green" | "yellow" | "orange" | "red" | ""`).
    pub alert: String,
    /// WGS84 longitude.
    pub longitude: f64,
    /// WGS84 latitude.
    pub latitude: f64,
    /// Depth in km.
    pub depth_km: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EarthquakesSnapshot {
    /// Events ranked by magnitude descending.
    pub rows: Vec<EarthquakeRow>,
    /// Total events the upstream returned (before filtering).
    pub total_events: usize,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled event — mirrors `pellucid_streams::EarthquakeEvent`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedEvent {
    /// Magnitude.
    pub magnitude: f64,
    /// Place description.
    pub place: String,
    /// Time (wall-clock ms).
    pub time_ms: i64,
    /// Event-page URL.
    pub url: String,
    /// Tsunami flag.
    pub tsunami: i64,
    /// PAGER alert level.
    pub alert: String,
    /// Longitude.
    pub longitude: f64,
    /// Latitude.
    pub latitude: f64,
    /// Depth in km.
    pub depth_km: f64,
}

/// DI trait — wraps `pellucid_streams::UsgsEarthquakesClient::fetch_feed`.
#[async_trait]
pub trait EarthquakesFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the last-24h all-magnitudes feed.
    async fn fetch_all_day(
        &self,
    ) -> Result<Vec<FetchedEvent>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`ClimateSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn EarthquakesFetcher,
    config: &EarthquakesConfig,
) -> Result<PublishOutcome, ClimateSeederError> {
    let fetched = fetcher
        .fetch_all_day()
        .await
        .map_err(|e| ClimateSeederError::Upstream(e.to_string()))?;
    let total_events = fetched.len();
    let mut rows: Vec<EarthquakeRow> = fetched
        .into_iter()
        .filter(|e| e.magnitude >= config.min_magnitude)
        .map(|e| EarthquakeRow {
            magnitude: e.magnitude,
            place: e.place,
            time_ms: e.time_ms,
            url: e.url,
            tsunami: e.tsunami,
            alert: e.alert,
            longitude: e.longitude,
            latitude: e.latitude,
            depth_km: e.depth_km,
        })
        .collect();
    if rows.is_empty() {
        return Err(ClimateSeederError::EmptyUpstream);
    }
    rows.sort_by(|a, b| {
        b.magnitude
            .partial_cmp(&a.magnitude)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = EarthquakesSnapshot {
        rows,
        total_events,
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
        atomic_publish(pool, "seismology", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        events: Vec<FetchedEvent>,
    }

    #[async_trait]
    impl EarthquakesFetcher for StaticFetcher {
        async fn fetch_all_day(
            &self,
        ) -> Result<Vec<FetchedEvent>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.events.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl EarthquakesFetcher for FailingFetcher {
        async fn fetch_all_day(
            &self,
        ) -> Result<Vec<FetchedEvent>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn quake(mag: f64, place: &str) -> FetchedEvent {
        FetchedEvent {
            magnitude: mag,
            place: place.into(),
            time_ms: 1_714_060_800_000,
            url: format!("https://earthquake.usgs.gov/{place}"),
            tsunami: 0,
            alert: "green".into(),
            longitude: -118.0,
            latitude: 34.0,
            depth_km: 5.0,
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "seismology:recent-quakes:24h:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_sorted_by_magnitude_desc() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![
                quake(2.0, "below threshold"), // dropped
                quake(5.4, "Hawaii"),
                quake(3.1, "California"),
                quake(6.2, "Alaska"),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &EarthquakesConfig::default())
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
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert!((rows[0].get("magnitude").unwrap().as_f64().unwrap() - 6.2).abs() < 1e-9);
        assert!((rows[1].get("magnitude").unwrap().as_f64().unwrap() - 5.4).abs() < 1e-9);
        assert!((rows[2].get("magnitude").unwrap().as_f64().unwrap() - 3.1).abs() < 1e-9);
        assert_eq!(
            parsed.pointer("/data/total_events").unwrap().as_u64(),
            Some(4)
        );
    }

    #[tokio::test]
    async fn run_cycle_all_below_threshold_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![quake(1.0, "x"), quake(1.5, "y")],
        };
        let err = run_cycle(&pool, &fetcher, &EarthquakesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ClimateSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &EarthquakesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ClimateSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![quake(5.4, "Hawaii")],
        };
        let _ = run_cycle(&pool, &fetcher, &EarthquakesConfig::default())
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

    #[tokio::test]
    async fn run_cycle_custom_min_magnitude_threshold() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![quake(4.0, "below"), quake(5.0, "above"), quake(7.0, "way above")],
        };
        let cfg = EarthquakesConfig { min_magnitude: 5.0 };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 2);
    }
}
