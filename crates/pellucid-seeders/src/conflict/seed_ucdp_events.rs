//! seed_ucdp_events — FAST-tier snapshot of recent UCDP GED
//! conflict events.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::conflict::ConflictSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "conflict:events-24h:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "ucdp-ged-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "conflict-ucdp";

/// Default page size.
pub const DEFAULT_PAGE_SIZE: u32 = 200;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct UcdpEventsConfig {
    /// Year to fetch (defaults to today's UTC year).
    pub year: u16,
    /// Server-side page size to request.
    pub page_size: u32,
}

impl Default for UcdpEventsConfig {
    fn default() -> Self {
        Self {
            year: today_year(),
            page_size: DEFAULT_PAGE_SIZE,
        }
    }
}

/// One event row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UcdpEventRow {
    /// GED event id.
    pub id: String,
    /// `YYYY-MM-DD` event start date.
    pub date_start: String,
    /// Country name.
    pub country: String,
    /// Side A.
    pub side_a: String,
    /// Side B.
    pub side_b: String,
    /// `1` state-based / `2` non-state / `3` one-sided.
    pub type_of_violence: i32,
    /// UCDP `best` fatality estimate.
    pub best_fatalities: i32,
    /// WGS84 latitude.
    pub latitude: f64,
    /// WGS84 longitude.
    pub longitude: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UcdpEventsSnapshot {
    /// Events ranked by best_fatalities desc.
    pub rows: Vec<UcdpEventRow>,
    /// Server-side total count for the query.
    pub total_count: u64,
    /// Calendar year the snapshot covers.
    pub year: u16,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled event — mirrors `pellucid_streams::UcdpEvent`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedUcdpEvent {
    /// GED event id.
    pub id: String,
    /// Event start date.
    pub date_start: String,
    /// Country.
    pub country: String,
    /// Side A.
    pub side_a: String,
    /// Side B.
    pub side_b: String,
    /// Violence type.
    pub type_of_violence: i32,
    /// Best fatality estimate.
    pub best_fatalities: i32,
    /// Latitude.
    pub latitude: f64,
    /// Longitude.
    pub longitude: f64,
}

/// Distilled page result.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedUcdpPage {
    /// Server-side total count.
    pub total_count: u64,
    /// Events on this page.
    pub events: Vec<FetchedUcdpEvent>,
}

/// DI trait — wraps `pellucid_streams::UcdpClient::fetch_ged_events`.
#[async_trait]
pub trait UcdpEventsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch one page of GED events.
    async fn fetch_ged_events(
        &self,
        year: u16,
        page_size: u32,
    ) -> Result<FetchedUcdpPage, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`ConflictSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn UcdpEventsFetcher,
    config: &UcdpEventsConfig,
) -> Result<PublishOutcome, ConflictSeederError> {
    let page = fetcher
        .fetch_ged_events(config.year, config.page_size)
        .await
        .map_err(|e| ConflictSeederError::Upstream(e.to_string()))?;
    if page.events.is_empty() {
        return Err(ConflictSeederError::EmptyUpstream);
    }
    let mut rows: Vec<UcdpEventRow> = page
        .events
        .into_iter()
        .map(|e| UcdpEventRow {
            id: e.id,
            date_start: e.date_start,
            country: e.country,
            side_a: e.side_a,
            side_b: e.side_b,
            type_of_violence: e.type_of_violence,
            best_fatalities: e.best_fatalities,
            latitude: e.latitude,
            longitude: e.longitude,
        })
        .collect();
    rows.sort_by(|a, b| b.best_fatalities.cmp(&a.best_fatalities));

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = UcdpEventsSnapshot {
        rows,
        total_count: page.total_count,
        year: config.year,
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

fn today_year() -> u16 {
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
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + i64::from(m <= 2)) as u16
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        page: FetchedUcdpPage,
    }

    #[async_trait]
    impl UcdpEventsFetcher for StaticFetcher {
        async fn fetch_ged_events(
            &self,
            _year: u16,
            _page_size: u32,
        ) -> Result<FetchedUcdpPage, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.page.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl UcdpEventsFetcher for FailingFetcher {
        async fn fetch_ged_events(
            &self,
            _year: u16,
            _page_size: u32,
        ) -> Result<FetchedUcdpPage, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn event(id: &str, country: &str, fatalities: i32) -> FetchedUcdpEvent {
        FetchedUcdpEvent {
            id: id.into(),
            date_start: "2024-04-25".into(),
            country: country.into(),
            side_a: "Side A".into(),
            side_b: "Side B".into(),
            type_of_violence: 1,
            best_fatalities: fatalities,
            latitude: 35.0,
            longitude: 38.0,
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "conflict:events-24h:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_sorted_by_fatalities_desc() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            page: FetchedUcdpPage {
                total_count: 1234,
                events: vec![
                    event("GED-1", "Syria", 5),
                    event("GED-2", "Yemen", 50),
                    event("GED-3", "Iraq", 12),
                ],
            },
        };
        let outcome = run_cycle(&pool, &fetcher, &UcdpEventsConfig::default())
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
        assert_eq!(rows[0].get("id").unwrap().as_str().unwrap(), "GED-2");
        assert_eq!(rows[1].get("id").unwrap().as_str().unwrap(), "GED-3");
        assert_eq!(rows[2].get("id").unwrap().as_str().unwrap(), "GED-1");
        assert_eq!(
            parsed.pointer("/data/total_count").unwrap().as_u64(),
            Some(1234)
        );
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            page: FetchedUcdpPage {
                total_count: 0,
                events: vec![],
            },
        };
        let err = run_cycle(&pool, &fetcher, &UcdpEventsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ConflictSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &UcdpEventsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ConflictSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            page: FetchedUcdpPage {
                total_count: 1,
                events: vec![event("GED-1", "Syria", 5)],
            },
        };
        let _ = run_cycle(&pool, &fetcher, &UcdpEventsConfig::default())
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
