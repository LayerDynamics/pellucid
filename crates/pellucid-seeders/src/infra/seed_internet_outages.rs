//! seed_internet_outages — FAST-tier snapshot of Cloudflare
//! Radar outage annotations.
//!
//! Cache key: `infrastructure:grid-stress:current:v1` — the
//! panel originally tracked grid stress; since Pellucid does
//! not have access to a free grid-stress API, this seeder
//! reuses the same panel slot for internet-outage events
//! (closest "infrastructure under stress" signal available
//! free).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::infra::InfraSeederError;

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "infrastructure:grid-stress:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "internet-outages-cloudflare-radar-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "infra-outages";

/// Default lookback window — 7 days.
pub const DEFAULT_DATE_RANGE: &str = "7d";

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct InternetOutagesConfig {
    /// Cloudflare Radar `dateRange` parameter.
    pub date_range: String,
}

impl Default for InternetOutagesConfig {
    fn default() -> Self {
        Self {
            date_range: DEFAULT_DATE_RANGE.to_string(),
        }
    }
}

/// One outage row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutageRow {
    /// Annotation UUID.
    pub uuid: String,
    /// `country` / `asn` / `region`.
    pub scope: String,
    /// Comma-joined location names.
    pub locations: String,
    /// Outage type.
    pub outage_type: String,
    /// Outage cause.
    pub outage_cause: String,
    /// Description.
    pub description: String,
    /// ISO-8601 start.
    pub start_date: String,
    /// ISO-8601 end (empty when ongoing).
    pub end_date: String,
    /// Linked URL.
    pub linked_url: String,
    /// Pre-computed `true` iff `end_date` is empty.
    pub ongoing: bool,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InternetOutagesSnapshot {
    /// Outages — newest-first.
    pub rows: Vec<OutageRow>,
    /// Echo of the date-range filter.
    pub date_range: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled annotation — mirrors `pellucid_streams::OutageAnnotation`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedOutage {
    /// UUID.
    pub uuid: String,
    /// Scope.
    pub scope: String,
    /// Locations.
    pub locations: String,
    /// Outage type.
    pub outage_type: String,
    /// Cause.
    pub outage_cause: String,
    /// Description.
    pub description: String,
    /// Start.
    pub start_date: String,
    /// End.
    pub end_date: String,
    /// Linked URL.
    pub linked_url: String,
}

/// DI trait — wraps `pellucid_streams::CloudflareRadarClient::fetch_outages`.
#[async_trait]
pub trait InternetOutagesFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch outage annotations.
    async fn fetch_outages(
        &self,
        date_range: &str,
    ) -> Result<Vec<FetchedOutage>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`InfraSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn InternetOutagesFetcher,
    config: &InternetOutagesConfig,
) -> Result<PublishOutcome, InfraSeederError> {
    let fetched = fetcher
        .fetch_outages(&config.date_range)
        .await
        .map_err(|e| InfraSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(InfraSeederError::EmptyUpstream);
    }
    let mut rows: Vec<OutageRow> = fetched
        .into_iter()
        .map(|o| OutageRow {
            ongoing: o.end_date.is_empty(),
            uuid: o.uuid,
            scope: o.scope,
            locations: o.locations,
            outage_type: o.outage_type,
            outage_cause: o.outage_cause,
            description: o.description,
            start_date: o.start_date,
            end_date: o.end_date,
            linked_url: o.linked_url,
        })
        .collect();
    rows.sort_by(|a, b| b.start_date.cmp(&a.start_date));

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = InternetOutagesSnapshot {
        rows,
        date_range: config.date_range.clone(),
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
        atomic_publish(pool, "infrastructure", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedOutage>,
    }

    #[async_trait]
    impl InternetOutagesFetcher for StaticFetcher {
        async fn fetch_outages(
            &self,
            _date_range: &str,
        ) -> Result<Vec<FetchedOutage>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    fn outage(uuid: &str, start: &str, end: Option<&str>) -> FetchedOutage {
        FetchedOutage {
            uuid: uuid.into(),
            scope: "country".into(),
            locations: "Sudan".into(),
            outage_type: "POWEROUTAGE".into(),
            outage_cause: "POWER".into(),
            description: format!("Outage {uuid}"),
            start_date: start.into(),
            end_date: end.unwrap_or("").into(),
            linked_url: "https://example.com".into(),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "infrastructure:grid-stress:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_pre_computes_ongoing_and_sorts_newest_first() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                outage("a", "2026-04-25T08:00:00Z", Some("2026-04-25T18:00:00Z")),
                outage("b", "2026-04-26T08:00:00Z", None),
                outage("c", "2026-04-24T08:00:00Z", Some("2026-04-24T20:00:00Z")),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &InternetOutagesConfig::default())
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
        assert_eq!(rows[0].get("uuid").unwrap().as_str().unwrap(), "b");
        assert_eq!(rows[0].get("ongoing").unwrap().as_bool(), Some(true));
        assert_eq!(rows[2].get("uuid").unwrap().as_str().unwrap(), "c");
        assert_eq!(rows[1].get("ongoing").unwrap().as_bool(), Some(false));
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &InternetOutagesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, InfraSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![outage("a", "2026-04-25T08:00:00Z", None)],
        };
        let _ = run_cycle(&pool, &fetcher, &InternetOutagesConfig::default())
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
