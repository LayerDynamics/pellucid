//! seed_acled — FAST-tier snapshot of recent ACLED events
//! grouped by primary actor, surfaced as the
//! "hot actors" panel.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::conflict::ConflictSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "conflict:hot-actors:24h:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "acled-hot-actors-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "conflict-acled";

/// Default ISO numeric basket — Syria, Iraq, Yemen, Iran,
/// Ukraine, Sudan, Gaza/Palestine, Lebanon.
pub const DEFAULT_ISO_CODES: &[u16] = &[760, 368, 887, 364, 804, 729, 275, 422];

/// Default record cap per cycle.
pub const DEFAULT_LIMIT: u32 = 500;

/// Default top-N actors kept in the snapshot.
pub const DEFAULT_TOP_ACTORS: usize = 25;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct AcledConfig {
    /// ISO 3166-1 numeric country codes.
    pub iso_codes: Vec<u16>,
    /// `YYYY-MM-DD` start date.
    pub start_date: String,
    /// `YYYY-MM-DD` end date.
    pub end_date: String,
    /// Max events to fetch per cycle.
    pub limit: u32,
    /// Top-N actors kept in the published snapshot.
    pub top_actors: usize,
}

impl AcledConfig {
    /// Build a config covering the default ISO basket + the
    /// last 7 days through today.
    #[must_use]
    pub fn default_for_today_utc() -> Self {
        let (y, m, d) = today_utc_ymd();
        let (sy, sm, sd) = subtract_days(y, m, d, 7);
        Self {
            iso_codes: DEFAULT_ISO_CODES.to_vec(),
            start_date: format!("{sy:04}-{sm:02}-{sd:02}"),
            end_date: format!("{y:04}-{m:02}-{d:02}"),
            limit: DEFAULT_LIMIT,
            top_actors: DEFAULT_TOP_ACTORS,
        }
    }
}

/// One actor row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActorRow {
    /// Actor name (ACLED `actor1`).
    pub actor: String,
    /// Number of events the actor appears in.
    pub event_count: u64,
    /// Total reported fatalities across those events.
    pub total_fatalities: i64,
    /// Country breakdown (country → event count).
    pub country_breakdown: Vec<(String, u64)>,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AcledSnapshot {
    /// Top-N actors ranked by event count desc.
    pub rows: Vec<ActorRow>,
    /// Total events the upstream returned.
    pub total_events: usize,
    /// Echo of date range.
    pub date_range: (String, String),
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled event — mirrors `pellucid_streams::AcledEvent`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedAcledEvent {
    /// Event id.
    pub event_id_cnty: String,
    /// Event date.
    pub event_date: String,
    /// Event type.
    pub event_type: String,
    /// Sub-event type.
    pub sub_event_type: String,
    /// Primary actor.
    pub actor1: String,
    /// Secondary actor.
    pub actor2: String,
    /// Country.
    pub country: String,
    /// Latitude.
    pub latitude: f64,
    /// Longitude.
    pub longitude: f64,
    /// Fatalities.
    pub fatalities: i64,
}

/// DI trait — wraps `pellucid_streams::AcledClient::fetch_events`.
#[async_trait]
pub trait AcledFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch events for the supplied ISO codes + date range.
    async fn fetch_events(
        &self,
        iso_codes: &[u16],
        start_date: &str,
        end_date: &str,
        limit: u32,
    ) -> Result<Vec<FetchedAcledEvent>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`ConflictSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn AcledFetcher,
    config: &AcledConfig,
) -> Result<PublishOutcome, ConflictSeederError> {
    let fetched = fetcher
        .fetch_events(
            &config.iso_codes,
            &config.start_date,
            &config.end_date,
            config.limit,
        )
        .await
        .map_err(|e| ConflictSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(ConflictSeederError::EmptyUpstream);
    }
    let total_events = fetched.len();

    // Roll up per-actor stats.
    use std::collections::BTreeMap;
    type ActorStats = (u64, i64, BTreeMap<String, u64>);
    let mut by_actor: BTreeMap<String, ActorStats> = BTreeMap::new();
    for ev in fetched {
        if ev.actor1.is_empty() {
            continue;
        }
        let entry = by_actor
            .entry(ev.actor1.clone())
            .or_insert((0, 0, BTreeMap::new()));
        entry.0 += 1;
        entry.1 += ev.fatalities;
        *entry.2.entry(ev.country.clone()).or_insert(0) += 1;
    }
    if by_actor.is_empty() {
        return Err(ConflictSeederError::EmptyUpstream);
    }

    let mut rows: Vec<ActorRow> = by_actor
        .into_iter()
        .map(
            |(actor, (event_count, total_fatalities, breakdown))| ActorRow {
                actor,
                event_count,
                total_fatalities,
                country_breakdown: breakdown.into_iter().collect(),
            },
        )
        .collect();
    rows.sort_by(|a, b| b.event_count.cmp(&a.event_count));
    rows.truncate(config.top_actors);

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = AcledSnapshot {
        rows,
        total_events,
        date_range: (config.start_date.clone(), config.end_date.clone()),
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
    let outcome = atomic_publish(pool, "conflict", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn today_utc_ymd() -> (u16, u8, u8) {
    let secs = pellucid_core::now_ms() / 1000;
    let days = secs / 86_400 + 719_468;
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
    let year = (y + i64::from(m <= 2)) as u16;
    (year, m as u8, d as u8)
}

const fn subtract_days(year: u16, month: u8, day: u8, n: u32) -> (u16, u8, u8) {
    let mut y = year;
    let mut m = month;
    let mut d = day as u32;
    let mut remaining = n;
    while remaining > 0 {
        if d > 1 {
            d -= 1;
        } else if m > 1 {
            m -= 1;
            d = days_in_month(y, m) as u32;
        } else {
            y -= 1;
            m = 12;
            d = 31;
        }
        remaining -= 1;
    }
    (y, m, d as u8)
}

const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let y = year as u32;
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        events: Vec<FetchedAcledEvent>,
    }

    #[async_trait]
    impl AcledFetcher for StaticFetcher {
        async fn fetch_events(
            &self,
            _iso_codes: &[u16],
            _start_date: &str,
            _end_date: &str,
            _limit: u32,
        ) -> Result<Vec<FetchedAcledEvent>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.events.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl AcledFetcher for FailingFetcher {
        async fn fetch_events(
            &self,
            _iso_codes: &[u16],
            _start_date: &str,
            _end_date: &str,
            _limit: u32,
        ) -> Result<Vec<FetchedAcledEvent>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn ev(id: &str, actor: &str, country: &str, fatalities: i64) -> FetchedAcledEvent {
        FetchedAcledEvent {
            event_id_cnty: id.into(),
            event_date: "2024-04-25".into(),
            event_type: "Battles".into(),
            sub_event_type: "Armed clash".into(),
            actor1: actor.into(),
            actor2: "Other".into(),
            country: country.into(),
            latitude: 35.0,
            longitude: 38.0,
            fatalities,
        }
    }

    fn config_for(events: usize) -> AcledConfig {
        AcledConfig {
            iso_codes: vec![760],
            start_date: "2024-04-25".into(),
            end_date: "2024-05-04".into(),
            limit: events as u32,
            top_actors: DEFAULT_TOP_ACTORS,
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "conflict:hot-actors:24h:v1");
    }

    #[test]
    fn default_for_today_utc_yields_seven_day_window() {
        let cfg = AcledConfig::default_for_today_utc();
        assert_eq!(cfg.iso_codes.len(), 8);
        assert_eq!(cfg.start_date.len(), 10);
        assert_eq!(cfg.end_date.len(), 10);
        assert_eq!(cfg.top_actors, DEFAULT_TOP_ACTORS);
    }

    #[test]
    fn subtract_days_simple() {
        assert_eq!(subtract_days(2026, 5, 4, 1), (2026, 5, 3));
        assert_eq!(subtract_days(2026, 5, 1, 1), (2026, 4, 30));
        assert_eq!(subtract_days(2026, 1, 1, 1), (2025, 12, 31));
        assert_eq!(subtract_days(2024, 3, 1, 1), (2024, 2, 29));
        assert_eq!(subtract_days(2026, 5, 8, 7), (2026, 5, 1));
    }

    #[tokio::test]
    async fn run_cycle_rolls_up_actor_stats() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![
                ev("e1", "Hamas", "Palestine", 5),
                ev("e2", "Hamas", "Israel", 3),
                ev("e3", "ISIS", "Syria", 12),
                ev("e4", "ISIS", "Iraq", 7),
                ev("e5", "ISIS", "Iraq", 4),
                ev("e6", "Houthis", "Yemen", 2),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &config_for(6)).await.unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 3);
        // ISIS: 3 events; Hamas: 2; Houthis: 1.
        assert_eq!(rows[0].get("actor").unwrap().as_str().unwrap(), "ISIS");
        assert_eq!(rows[0].get("event_count").unwrap().as_u64(), Some(3));
        assert_eq!(rows[0].get("total_fatalities").unwrap().as_i64(), Some(23));
        assert_eq!(rows[1].get("actor").unwrap().as_str().unwrap(), "Hamas");
        assert_eq!(rows[2].get("actor").unwrap().as_str().unwrap(), "Houthis");
        assert_eq!(
            parsed.pointer("/data/total_events").unwrap().as_u64(),
            Some(6)
        );
    }

    #[tokio::test]
    async fn run_cycle_drops_actor_with_empty_name() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![ev("e1", "", "Syria", 5), ev("e2", "ISIS", "Syria", 3)],
        };
        let _ = run_cycle(&pool, &fetcher, &config_for(2)).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("actor").unwrap().as_str().unwrap(), "ISIS");
    }

    #[tokio::test]
    async fn run_cycle_truncates_to_top_actors() {
        let pool = open_in_memory().await.unwrap();
        let mut events: Vec<FetchedAcledEvent> = Vec::new();
        for i in 0..30 {
            for _ in 0..(i + 1) {
                events.push(ev("e", &format!("actor-{i:02}"), "X", 1));
            }
        }
        let fetcher = StaticFetcher { events };
        let cfg = AcledConfig {
            iso_codes: vec![760],
            start_date: "2024-04-25".into(),
            end_date: "2024-05-04".into(),
            limit: 1000,
            top_actors: 5,
        };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 5);
        // Top actor must be `actor-29` (30 events).
        assert_eq!(rows[0].get("actor").unwrap().as_str().unwrap(), "actor-29");
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { events: vec![] };
        let err = run_cycle(&pool, &fetcher, &config_for(0))
            .await
            .unwrap_err();
        assert!(matches!(err, ConflictSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &config_for(0))
            .await
            .unwrap_err();
        assert!(matches!(err, ConflictSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            events: vec![ev("e1", "ISIS", "Syria", 5)],
        };
        let _ = run_cycle(&pool, &fetcher, &config_for(1)).await.unwrap();
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
