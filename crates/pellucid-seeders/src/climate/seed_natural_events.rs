//! seed_natural_events — FAST-tier snapshot of open natural
//! events (volcanoes, severe storms, floods, droughts) from
//! NASA EONET.
//!
//! The published cache slot is named for volcanoes (the most
//! tracked category in the panel) but the seeder publishes
//! every open EONET event in the lookback window so the panel
//! can split by category client-side.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::climate::ClimateSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "natural:volcano-feed:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "natural-events-eonet-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "natural-events";

/// Default lookback window in days.
pub const DEFAULT_DAYS: u32 = 7;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct NaturalEventsConfig {
    /// Lookback window passed to EONET.
    pub days: u32,
    /// Optional category filter (e.g. `Some("volcanoes")`).
    /// `None` fetches every category.
    pub category: Option<String>,
}

impl Default for NaturalEventsConfig {
    fn default() -> Self {
        Self {
            days: DEFAULT_DAYS,
            category: None,
        }
    }
}

/// One event row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NaturalEventRow {
    /// EONET event id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Permalink.
    pub link: String,
    /// Comma-joined category titles.
    pub categories: String,
    /// Latest geometry (when present).
    pub latest_geometry: Option<EventGeometryRow>,
}

/// Last reported geometry for an event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventGeometryRow {
    /// ISO-8601 timestamp.
    pub date: String,
    /// `Point` (the seeder discards polygons; see `seed_natural_events`).
    pub kind: String,
    /// Longitude.
    pub longitude: f64,
    /// Latitude.
    pub latitude: f64,
    /// Magnitude value (acres, km, m, etc.).
    pub magnitude_value: f64,
    /// Magnitude unit string.
    pub magnitude_unit: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NaturalEventsSnapshot {
    /// Open events — one row per upstream event.
    pub rows: Vec<NaturalEventRow>,
    /// Lookback window (days) the seeder asked for.
    pub days: u32,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled event — mirrors `pellucid_streams::NaturalEvent`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedEvent {
    /// EONET id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Permalink.
    pub link: String,
    /// Joined category titles.
    pub categories: String,
    /// Latest point geometry.
    pub latest_geometry: Option<FetchedGeometry>,
}

/// Distilled point geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedGeometry {
    /// ISO-8601 date.
    pub date: String,
    /// Geometry kind.
    pub kind: String,
    /// Longitude.
    pub longitude: f64,
    /// Latitude.
    pub latitude: f64,
    /// Magnitude value.
    pub magnitude_value: f64,
    /// Magnitude unit.
    pub magnitude_unit: String,
}

/// DI trait — wraps `pellucid_streams::NasaEonetClient::fetch_events`.
#[async_trait]
pub trait NaturalEventsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch open events.
    async fn fetch_events(
        &self,
        category: Option<&str>,
        days: u32,
    ) -> Result<Vec<FetchedEvent>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`ClimateSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn NaturalEventsFetcher,
    config: &NaturalEventsConfig,
) -> Result<PublishOutcome, ClimateSeederError> {
    let fetched = fetcher
        .fetch_events(config.category.as_deref(), config.days)
        .await
        .map_err(|e| ClimateSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(ClimateSeederError::EmptyUpstream);
    }

    let rows: Vec<NaturalEventRow> = fetched
        .into_iter()
        .map(|e| NaturalEventRow {
            id: e.id,
            title: e.title,
            link: e.link,
            categories: e.categories,
            latest_geometry: e.latest_geometry.map(|g| EventGeometryRow {
                date: g.date,
                kind: g.kind,
                longitude: g.longitude,
                latitude: g.latitude,
                magnitude_value: g.magnitude_value,
                magnitude_unit: g.magnitude_unit,
            }),
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = NaturalEventsSnapshot {
        rows,
        days: config.days,
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
    let outcome = atomic_publish(pool, "natural", CACHE_KEY, &envelope, TTL).await?;
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
        last_category: std::sync::Mutex<Option<String>>,
        last_days: std::sync::Mutex<u32>,
    }

    #[async_trait]
    impl NaturalEventsFetcher for StaticFetcher {
        async fn fetch_events(
            &self,
            category: Option<&str>,
            days: u32,
        ) -> Result<Vec<FetchedEvent>, Box<dyn std::error::Error + Send + Sync>> {
            *self.last_category.lock().unwrap() = category.map(str::to_string);
            *self.last_days.lock().unwrap() = days;
            Ok(self.events.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl NaturalEventsFetcher for FailingFetcher {
        async fn fetch_events(
            &self,
            _category: Option<&str>,
            _days: u32,
        ) -> Result<Vec<FetchedEvent>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn event(id: &str, cat: &str, with_geom: bool) -> FetchedEvent {
        FetchedEvent {
            id: id.into(),
            title: format!("{cat} — test"),
            link: format!("https://eonet.gsfc.nasa.gov/events/{id}"),
            categories: cat.into(),
            latest_geometry: if with_geom {
                Some(FetchedGeometry {
                    date: "2026-05-04T18:00:00Z".into(),
                    kind: "Point".into(),
                    longitude: -122.5,
                    latitude: 41.7,
                    magnitude_value: 12500.0,
                    magnitude_unit: "acres".into(),
                })
            } else {
                None
            },
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "natural:volcano-feed:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_geometry_when_present() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![
                event("EONET_1", "Wildfires", true),
                event("EONET_2", "Volcanoes", false),
            ],
            last_category: std::sync::Mutex::new(None),
            last_days: std::sync::Mutex::new(0),
        };
        let outcome = run_cycle(&pool, &fetcher, &NaturalEventsConfig::default())
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
        assert_eq!(rows.len(), 2);
        assert!(rows[0].get("latest_geometry").unwrap().is_object());
        assert!(rows[1].get("latest_geometry").unwrap().is_null());
        assert_eq!(parsed.pointer("/data/days").unwrap().as_u64(), Some(7));
    }

    #[tokio::test]
    async fn run_cycle_passes_category_to_fetcher() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![event("EONET_1", "Volcanoes", true)],
            last_category: std::sync::Mutex::new(None),
            last_days: std::sync::Mutex::new(0),
        };
        let cfg = NaturalEventsConfig {
            days: 14,
            category: Some("volcanoes".into()),
        };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        assert_eq!(
            fetcher.last_category.lock().unwrap().as_deref(),
            Some("volcanoes")
        );
        assert_eq!(*fetcher.last_days.lock().unwrap(), 14);
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![],
            last_category: std::sync::Mutex::new(None),
            last_days: std::sync::Mutex::new(0),
        };
        let err = run_cycle(&pool, &fetcher, &NaturalEventsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ClimateSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &NaturalEventsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ClimateSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![event("EONET_1", "Wildfires", true)],
            last_category: std::sync::Mutex::new(None),
            last_days: std::sync::Mutex::new(0),
        };
        let _ = run_cycle(&pool, &fetcher, &NaturalEventsConfig::default())
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
