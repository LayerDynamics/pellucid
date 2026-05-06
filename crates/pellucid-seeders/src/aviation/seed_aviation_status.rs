//! seed_aviation_status — FAST-tier snapshot of a flight
//! watchlist via aviationstack (SPEC-001 §17.7).
//!
//! The webview's `AviationPanel` ships an "above the fold"
//! tile of high-traffic / high-watch flights. Rather than
//! waiting for a user to hit `/api/aviation/v1/get-flight-status`
//! per flight, this seeder fetches the watchlist once per
//! cycle and writes one snapshot to
//! `aviation:breaking-incidents:v1`.
//!
//! The watchlist is configurable; the default targets eight
//! large-volume routes the panel highlights:
//!
//! | Flight | Origin | Carrier-route                     |
//! |--------|--------|-----------------------------------|
//! | AA100  | JFK    | American Airlines transcontinental|
//! | UA1    | EWR    | United premium transcon           |
//! | DL1    | JFK    | Delta One JFK→LAX                 |
//! | BA178  | JFK    | British Airways JFK→LHR           |
//! | AF23   | JFK    | Air France JFK→CDG                |
//! | LH401  | JFK    | Lufthansa JFK→FRA                 |
//! | EK202  | JFK    | Emirates JFK→DXB                  |
//! | QF12   | LAX    | Qantas LAX→SYD                    |

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::aviation::AviationSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in
/// `pellucid_handlers::bootstrap::keys::FAST_KEYS`.
pub const CACHE_KEY: &str = "aviation:breaking-incidents:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "aviation-status-aviationstack-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "aviation-status";

/// One watchlist entry: `(flight_iata, dep_iata)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchEntry {
    /// IATA flight number (e.g. `"AA100"`).
    pub flight: &'static str,
    /// IATA departure code (e.g. `"JFK"`).
    pub dep: &'static str,
}

/// Default watchlist — eight large-volume routes.
pub const DEFAULT_WATCHLIST: &[WatchEntry] = &[
    WatchEntry {
        flight: "AA100",
        dep: "JFK",
    },
    WatchEntry {
        flight: "UA1",
        dep: "EWR",
    },
    WatchEntry {
        flight: "DL1",
        dep: "JFK",
    },
    WatchEntry {
        flight: "BA178",
        dep: "JFK",
    },
    WatchEntry {
        flight: "AF23",
        dep: "JFK",
    },
    WatchEntry {
        flight: "LH401",
        dep: "JFK",
    },
    WatchEntry {
        flight: "EK202",
        dep: "JFK",
    },
    WatchEntry {
        flight: "QF12",
        dep: "LAX",
    },
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct AviationStatusConfig {
    /// Watchlist — `(flight, dep)` pairs to fetch each cycle.
    pub watchlist: Vec<(String, String)>,
    /// Date the seeder asks the upstream about. Production
    /// passes today (UTC); tests pass a fixed string for
    /// deterministic wiremock matching.
    pub date: String,
}

impl AviationStatusConfig {
    /// Build a config from the default watchlist + today's UTC
    /// date in `YYYY-MM-DD` form.
    #[must_use]
    pub fn default_for_today_utc() -> Self {
        Self {
            watchlist: DEFAULT_WATCHLIST
                .iter()
                .map(|e| (e.flight.to_string(), e.dep.to_string()))
                .collect(),
            date: today_utc(),
        }
    }
}

/// One status row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AviationStatusRow {
    /// IATA flight number (echoed from the watchlist).
    pub flight: String,
    /// Origin IATA code.
    pub origin: String,
    /// Destination IATA code (from the upstream).
    pub destination: String,
    /// Flight status (`"scheduled" | "active" | "landed" | …`).
    pub status: String,
    /// Scheduled departure time as the upstream reported it.
    pub scheduled_departure: String,
    /// Scheduled arrival time as the upstream reported it.
    pub scheduled_arrival: String,
    /// Departure gate (when published).
    pub departure_gate: Option<String>,
    /// Arrival gate (when published).
    pub arrival_gate: Option<String>,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AviationStatusSnapshot {
    /// One row per watchlist flight the upstream had data for.
    pub rows: Vec<AviationStatusRow>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// DI trait — implemented by an adapter wrapping
/// `pellucid_streams::AviationstackClient::fetch_flight`.
#[async_trait]
pub trait AviationStatusFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch one flight's status. Returns `Ok(None)` when the
    /// upstream's `data[]` is empty (legitimate "no such
    /// flight on this date").
    async fn fetch_flight(
        &self,
        flight: &str,
        date: &str,
        origin: &str,
    ) -> Result<Option<FetchedFlightStatus>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Distilled flight-status — mirrors
/// `pellucid_handlers::generated::aviation::v1::FlightStatus`
/// without leaking the codegen type into this crate.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedFlightStatus {
    /// IATA flight number as upstream returned it.
    pub flight: String,
    /// Departure IATA.
    pub origin: String,
    /// Arrival IATA.
    pub destination: String,
    /// Flight status.
    pub status: String,
    /// Scheduled departure.
    pub scheduled_departure: String,
    /// Scheduled arrival.
    pub scheduled_arrival: String,
    /// Departure gate.
    pub departure_gate: Option<String>,
    /// Arrival gate.
    pub arrival_gate: Option<String>,
}

/// Run one cycle: fetch each watchlist flight in series
/// (aviationstack rate-limits on the access_key, so parallel
/// fan-out can trip 429s on smaller plans), assemble snapshot,
/// atomic-publish.
///
/// # Errors
/// See [`AviationSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn AviationStatusFetcher,
    config: &AviationStatusConfig,
) -> Result<PublishOutcome, AviationSeederError> {
    let mut rows: Vec<AviationStatusRow> = Vec::with_capacity(config.watchlist.len());
    for (flight, dep) in &config.watchlist {
        let result = fetcher
            .fetch_flight(flight, &config.date, dep)
            .await
            .map_err(|e| AviationSeederError::Upstream(e.to_string()))?;
        if let Some(s) = result {
            rows.push(AviationStatusRow {
                flight: s.flight,
                origin: s.origin,
                destination: s.destination,
                status: s.status,
                scheduled_departure: s.scheduled_departure,
                scheduled_arrival: s.scheduled_arrival,
                departure_gate: s.departure_gate,
                arrival_gate: s.arrival_gate,
            });
        }
    }
    if rows.is_empty() {
        return Err(AviationSeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = AviationStatusSnapshot {
        rows,
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
    let outcome = atomic_publish(pool, "aviation", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

/// Today's UTC date as `YYYY-MM-DD`. Derived from
/// `pellucid_core::now_ms()` so the test rig can pin it.
fn today_utc() -> String {
    let secs = pellucid_core::now_ms() / 1000;
    let days_since_epoch = secs / 86_400;
    let (year, month, day) = epoch_days_to_ymd(days_since_epoch);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Convert a count of days since `1970-01-01` into a
/// `(year, month, day)` triple. Pure arithmetic — no chrono
/// dep needed for a single date conversion.
fn epoch_days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Algorithm from Howard Hinnant's `civil_from_days`
    // (https://howardhinnant.github.io/date_algorithms.html).
    let days = days + 719_468;
    let era = if days >= 0 {
        days / 146_097
    } else {
        (days - 146_096) / 146_097
    };
    let doe = (days - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year_i32 = (y + i64::from(m <= 2)) as i32;
    (year_i32, m as u32, d as u32)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        responses: std::collections::HashMap<String, FetchedFlightStatus>,
    }

    #[async_trait]
    impl AviationStatusFetcher for StaticFetcher {
        async fn fetch_flight(
            &self,
            flight: &str,
            _date: &str,
            _origin: &str,
        ) -> Result<Option<FetchedFlightStatus>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.responses.get(flight).cloned())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl AviationStatusFetcher for FailingFetcher {
        async fn fetch_flight(
            &self,
            _flight: &str,
            _date: &str,
            _origin: &str,
        ) -> Result<Option<FetchedFlightStatus>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn flight(flight: &str, origin: &str, dest: &str, status: &str) -> FetchedFlightStatus {
        FetchedFlightStatus {
            flight: flight.into(),
            origin: origin.into(),
            destination: dest.into(),
            status: status.into(),
            scheduled_departure: "2026-05-04T12:00:00Z".into(),
            scheduled_arrival: "2026-05-04T15:30:00Z".into(),
            departure_gate: Some("A12".into()),
            arrival_gate: None,
        }
    }

    fn config_for_two_flights() -> AviationStatusConfig {
        AviationStatusConfig {
            watchlist: vec![("AA100".into(), "JFK".into()), ("UA1".into(), "EWR".into())],
            date: "2026-05-04".into(),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "aviation:breaking-incidents:v1");
    }

    #[test]
    fn ttl_is_fast_tier() {
        assert_eq!(TTL, Duration::from_secs(60));
    }

    #[test]
    fn default_watchlist_has_eight_entries() {
        assert_eq!(DEFAULT_WATCHLIST.len(), 8);
    }

    #[test]
    fn epoch_days_to_ymd_known_dates() {
        assert_eq!(epoch_days_to_ymd(0), (1970, 1, 1));
        assert_eq!(epoch_days_to_ymd(31), (1970, 2, 1));
        assert_eq!(epoch_days_to_ymd(365), (1971, 1, 1));
        // 2024-02-29 (leap year). Days from 1970-01-01:
        // 54 years * 365 + 14 leap days (1972…2024) - 1 (we
        // include 2024-02-29) = 19_710 + 14 = 19_777 - 1 = ...
        // easier sanity: 2026-05-04
        assert_eq!(epoch_days_to_ymd(20577), (2026, 5, 4));
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_two_flights() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert("AA100".into(), flight("AA100", "JFK", "LAX", "active"));
        responses.insert("UA1".into(), flight("UA1", "EWR", "SFO", "scheduled"));
        let fetcher = StaticFetcher { responses };
        let outcome = run_cycle(&pool, &fetcher, &config_for_two_flights())
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
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("flight").unwrap().as_str().unwrap(), "AA100");
        assert_eq!(rows[0].get("status").unwrap().as_str().unwrap(), "active");
    }

    #[tokio::test]
    async fn run_cycle_drops_flights_with_no_upstream_data() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert("AA100".into(), flight("AA100", "JFK", "LAX", "active"));
        // UA1 omitted.
        let fetcher = StaticFetcher { responses };
        let outcome = run_cycle(&pool, &fetcher, &config_for_two_flights())
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
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn run_cycle_no_upstream_hits_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            responses: std::collections::HashMap::new(),
        };
        let err = run_cycle(&pool, &fetcher, &config_for_two_flights())
            .await
            .unwrap_err();
        assert!(matches!(err, AviationSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &config_for_two_flights())
            .await
            .unwrap_err();
        assert!(matches!(err, AviationSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert("AA100".into(), flight("AA100", "JFK", "LAX", "active"));
        let fetcher = StaticFetcher { responses };
        let _ = run_cycle(
            &pool,
            &fetcher,
            &AviationStatusConfig {
                watchlist: vec![("AA100".into(), "JFK".into())],
                date: "2026-05-04".into(),
            },
        )
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

    #[test]
    fn default_for_today_utc_returns_iso_date() {
        let cfg = AviationStatusConfig::default_for_today_utc();
        // Sanity: YYYY-MM-DD shape (10 chars, 2 hyphens).
        assert_eq!(cfg.date.len(), 10);
        assert_eq!(cfg.date.chars().filter(|c| *c == '-').count(), 2);
        assert_eq!(cfg.watchlist.len(), DEFAULT_WATCHLIST.len());
    }
}
