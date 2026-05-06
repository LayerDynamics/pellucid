//! Theater-posture seeder — H3 FIX (SPEC-001 §17.7).
//!
//! The original WorldMonitor relay's theater-posture seeder
//! posted to its own gateway via `http://127.0.0.1:<port>` to
//! pull OpenSky data — a localhost loopback that:
//!
//! 1. Forced a 30 s startup-delay race (the seeder ran before
//!    the gateway was bound).
//! 2. Burned a TCP connection + JSON encode/decode round-trip
//!    per cycle.
//! 3. Routed cache reads through the public-facing rate-limit
//!    layer (counting toward the seeder's own rate budget).
//!
//! The H3 fix per SPEC-001 §17.7: the seeder calls the streams
//! client **directly in-process** via dependency injection. No
//! `reqwest` to localhost, no HTTP server started by the seeder
//! itself, no rate-limit double-counting.
//!
//! The seeder produces a `military:theater-posture:current:v1`
//! cache row at the FAST tier so the military panel's
//! cold-start hydration sees fresh data.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishError, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};

/// SPEC-001 §17.7 cache key for the theater-posture seeder.
pub const CACHE_KEY: &str = "military:theater-posture:current:v1";

/// Cache tier — FAST per SPEC-001 §17.7. The webview's military
/// panel polls this every 5 minutes; the FAST tier's 60-second
/// `s-maxage` matches the seeder's own cycle (one fresh write
/// per minute is enough).
pub const TTL: Duration = Duration::from_secs(60);

/// Seeder version — bumped when the bbox set or the response
/// shape changes so dashboards can spot drift through
/// `seed_meta.source_version`.
pub const SOURCE_VERSION: &str = "theater-posture-v1";

/// Cascade group tag — tells the relay's `/health` cascade that
/// this seeder + the panel reading it form one observability
/// unit.
pub const CASCADE_GROUP: &str = "theater-posture";

/// The four watch theaters the seeder polls per cycle. Coords
/// are `(lamin, lomin, lamax, lomax)` per OpenSky.
pub const THEATER_BOXES: &[TheaterBox] = &[
    TheaterBox {
        name: "europe",
        bbox: (35.0, -10.0, 70.0, 40.0),
    },
    TheaterBox {
        name: "middle-east",
        bbox: (10.0, 25.0, 45.0, 65.0),
    },
    TheaterBox {
        name: "western-pacific",
        bbox: (-10.0, 100.0, 50.0, 160.0),
    },
    TheaterBox {
        name: "north-america",
        bbox: (15.0, -130.0, 55.0, -65.0),
    },
];

/// A single watch theater.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TheaterBox {
    /// Slug used in the seeded payload (`europe`, `middle-east`,
    /// …).
    pub name: &'static str,
    /// `(lamin, lomin, lamax, lomax)` per OpenSky REST.
    pub bbox: (f64, f64, f64, f64),
}

impl Eq for TheaterBox {}

/// What the seeder writes to the cache. The military panel
/// renders this verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TheaterPostureSnapshot {
    /// Per-theater contact counts.
    pub theaters: Vec<TheaterReading>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// One theater's reading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TheaterReading {
    /// Theater slug (`europe`, etc.).
    pub theater: String,
    /// Number of aircraft tracked in the bbox at fetch time.
    /// `0` when the upstream returned negative/empty.
    pub aircraft_count: u32,
    /// `true` when the upstream returned data; `false` when the
    /// negative sentinel / cooldown short-circuited the fetch.
    pub fresh: bool,
}

/// Errors the seeder can surface.
#[derive(Debug, Error)]
pub enum SeederError {
    /// Underlying upstream error from the streams client.
    #[error("upstream: {0}")]
    Upstream(String),
    /// Atomic-publish failure.
    #[error("publish: {0}")]
    Publish(#[from] PublishError),
}

/// Trait the seeder consumes — keeps `pellucid-seeders` from
/// depending on `pellucid-streams` directly (which would cycle
/// once T3.10 wires the relay binary). Production hands in an
/// adapter wrapping `pellucid_streams::OpenSkyClient`; tests
/// hand in a wiremock-driven adapter.
#[async_trait]
pub trait OpenSkyBoxFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch state vectors inside `bbox`. Returns `Ok(None)`
    /// when the upstream had nothing fresh (negative sentinel
    /// or cooldown).
    async fn fetch_box(
        &self,
        bbox: (f64, f64, f64, f64),
    ) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle of the theater-posture seeder. Iterates over
/// every theater, calls the upstream **directly** (no HTTP),
/// assembles the snapshot, and atomic-publishes to the cache.
///
/// Returns the publish outcome so the scheduler / `/health`
/// cascade can read freshness metadata.
///
/// # Errors
/// See [`SeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn OpenSkyBoxFetcher,
) -> Result<PublishOutcome, SeederError> {
    let mut readings: Vec<TheaterReading> = Vec::with_capacity(THEATER_BOXES.len());
    for theater in THEATER_BOXES {
        let result = fetcher
            .fetch_box(theater.bbox)
            .await
            .map_err(|e| SeederError::Upstream(e.to_string()))?;
        let (count, fresh) = result
            .as_ref()
            .map(|v| (count_states(v), true))
            .unwrap_or((0, false));
        readings.push(TheaterReading {
            theater: theater.name.to_string(),
            aircraft_count: count,
            fresh,
        });
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = TheaterPostureSnapshot {
        theaters: readings,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(60_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.theaters.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };

    let outcome = atomic_publish(pool, "military", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

/// Count entries in the OpenSky `states` array. Tolerates the
/// upstream's `null` shape (returns 0).
pub fn count_states(value: &serde_json::Value) -> u32 {
    value
        .get("states")
        .and_then(|s| s.as_array())
        .map(|a| u32::try_from(a.len()).unwrap_or(u32::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct FixedFetcher {
        body: Option<serde_json::Value>,
    }

    #[async_trait]
    impl OpenSkyBoxFetcher for FixedFetcher {
        async fn fetch_box(
            &self,
            _bbox: (f64, f64, f64, f64),
        ) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.body.clone())
        }
    }

    #[derive(Debug)]
    struct CountingFetcher {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl OpenSkyBoxFetcher for CountingFetcher {
        async fn fetch_box(
            &self,
            _bbox: (f64, f64, f64, f64),
        ) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(
                serde_json::json!({ "states": [["abc", "AAL100  ", "US"]] }),
            ))
        }
    }

    #[test]
    fn theater_boxes_cover_four_known_regions() {
        let names: Vec<&str> = THEATER_BOXES.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec!["europe", "middle-east", "western-pacific", "north-america"]
        );
    }

    #[test]
    fn theater_boxes_have_valid_coordinates() {
        for t in THEATER_BOXES {
            let (lamin, lomin, lamax, lomax) = t.bbox;
            assert!(
                (-90.0..=90.0).contains(&lamin) && (-90.0..=90.0).contains(&lamax),
                "{}: latitudes out of range",
                t.name,
            );
            assert!(
                (-180.0..=180.0).contains(&lomin) && (-180.0..=180.0).contains(&lomax),
                "{}: longitudes out of range",
                t.name,
            );
            assert!(lamin < lamax, "{}: lamin >= lamax", t.name);
            assert!(lomin < lomax, "{}: lomin >= lomax", t.name);
        }
    }

    #[test]
    fn count_states_handles_array_null_missing() {
        let with = serde_json::json!({ "states": [["a"], ["b"], ["c"]] });
        assert_eq!(count_states(&with), 3);

        let null = serde_json::json!({ "states": null });
        assert_eq!(count_states(&null), 0);

        let missing = serde_json::json!({ "time": 1 });
        assert_eq!(count_states(&missing), 0);

        let empty = serde_json::json!({ "states": [] });
        assert_eq!(count_states(&empty), 0);
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_per_theater_readings() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = FixedFetcher {
            body: Some(serde_json::json!({ "states": [["a"], ["b"]] })),
        };
        let outcome = run_cycle(&pool, &fetcher).await.unwrap();
        assert!(outcome.bytes_written > 0);

        // Read the canonical row back + assert shape.
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let theaters = envelope
            .pointer("/data/theaters")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(theaters.len(), 4);
        for t in theaters {
            assert_eq!(t.get("aircraft_count").unwrap().as_u64(), Some(2));
            assert_eq!(t.get("fresh").unwrap().as_bool(), Some(true));
        }
    }

    #[tokio::test]
    async fn run_cycle_marks_negative_results_as_not_fresh() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = FixedFetcher { body: None };
        let outcome = run_cycle(&pool, &fetcher).await.unwrap();
        assert!(outcome.bytes_written > 0);

        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let theaters = envelope
            .pointer("/data/theaters")
            .unwrap()
            .as_array()
            .unwrap();
        for t in theaters {
            assert_eq!(t.get("fresh").unwrap().as_bool(), Some(false));
            assert_eq!(t.get("aircraft_count").unwrap().as_u64(), Some(0));
        }
    }

    #[tokio::test]
    async fn run_cycle_calls_upstream_once_per_theater() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = CountingFetcher {
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        // 4 theaters → exactly 4 upstream calls (no retries
        // when the mock returns Ok).
        assert_eq!(
            fetcher.calls.load(std::sync::atomic::Ordering::SeqCst),
            THEATER_BOXES.len()
        );
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta_with_cascade_group() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = FixedFetcher {
            body: Some(serde_json::json!({ "states": [] })),
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();

        let row: (String, String) = sqlx::query_as(
            "SELECT source_version, cascade_group FROM seed_meta WHERE cache_key = ?",
        )
        .bind(CACHE_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, SOURCE_VERSION);
        assert_eq!(row.1, CASCADE_GROUP);
    }

    #[test]
    fn cache_key_matches_spec() {
        // SPEC-001 §17.7 pins the canonical key.
        assert_eq!(CACHE_KEY, "military:theater-posture:current:v1");
    }

    #[test]
    fn ttl_matches_fast_tier() {
        // FAST tier = 60s s-maxage. Pinning here prevents an
        // accidental drift to a SLOW-tier value that would
        // mismatch the panel's polling cadence.
        assert_eq!(TTL, Duration::from_secs(60));
    }
}
