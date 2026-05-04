//! seed_notam — FAST-tier active-NOTAMs snapshot for the
//! airports the Pellucid airspace panel watches.
//!
//! The webview renders a roll-up of NOTAMs around the busiest
//! US airports + a rotating set of international hubs. The
//! seeder fetches them all in one round-trip via the FAA's
//! NOTAM Search REST API and publishes a snapshot grouped by
//! ICAO designator.
//!
//! Default designators (sized to fit one upstream call without
//! hitting the FAA's 100-NOTAM-per-response limit on busy
//! days):
//! - US east-coast: KJFK, KEWR, KLGA, KBOS, KDCA, KIAD
//! - US west-coast: KLAX, KSFO, KSEA, KDEN
//! - International: EGLL (LHR), EHAM (AMS), LFPG (CDG)

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::aviation::AviationSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "aviation:active-notams:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "notam-faa-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "aviation-notam";

/// Default ICAO designator basket.
pub const DEFAULT_DESIGNATORS: &[&str] = &[
    "KJFK", "KEWR", "KLGA", "KBOS", "KDCA", "KIAD",
    "KLAX", "KSFO", "KSEA", "KDEN",
    "EGLL", "EHAM", "LFPG",
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct NotamConfig {
    /// ICAO designators to fetch each cycle.
    pub designators: Vec<String>,
}

impl Default for NotamConfig {
    fn default() -> Self {
        Self {
            designators: DEFAULT_DESIGNATORS.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// One NOTAM row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotamRow {
    /// FAA NOTAM number (e.g. `"A1234/26"`).
    pub number: String,
    /// ICAO location.
    pub icao_location: String,
    /// Wall-clock seconds the NOTAM was issued.
    pub issue_date_unix: i64,
    /// Wall-clock seconds the NOTAM becomes effective.
    pub start_date_unix: i64,
    /// Wall-clock seconds the NOTAM expires.
    pub end_date_unix: i64,
    /// ICAO-format message body.
    pub message: String,
    /// `true` iff `start <= now <= end` at snapshot time.
    /// Pre-computed so the panel doesn't re-derive it.
    pub active_now: bool,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotamSnapshot {
    /// One row per upstream NOTAM, ordered as the FAA
    /// returned them (newest-first per the request sort).
    pub rows: Vec<NotamRow>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled NOTAM — mirrors `pellucid_streams::Notam`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedNotam {
    /// FAA NOTAM number.
    pub number: String,
    /// ICAO location.
    pub icao_location: String,
    /// Wall-clock seconds the NOTAM was issued.
    pub issue_date_unix: i64,
    /// Wall-clock seconds the NOTAM becomes effective.
    pub start_date_unix: i64,
    /// Wall-clock seconds the NOTAM expires.
    pub end_date_unix: i64,
    /// ICAO-format message body.
    pub message: String,
}

/// DI trait — wraps `pellucid_streams::FaaNotamClient::fetch_notams`.
#[async_trait]
pub trait NotamFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch NOTAMs around the supplied ICAO designators.
    async fn fetch_notams(
        &self,
        designators: &[&str],
    ) -> Result<Vec<FetchedNotam>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`AviationSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn NotamFetcher,
    config: &NotamConfig,
) -> Result<PublishOutcome, AviationSeederError> {
    let designator_refs: Vec<&str> =
        config.designators.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_notams(&designator_refs)
        .await
        .map_err(|e| AviationSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(AviationSeederError::EmptyUpstream);
    }

    let now_unix = pellucid_core::now_ms() / 1000;
    let rows: Vec<NotamRow> = fetched
        .into_iter()
        .map(|n| NotamRow {
            active_now: n.start_date_unix <= now_unix && now_unix <= n.end_date_unix,
            number: n.number,
            icao_location: n.icao_location,
            issue_date_unix: n.issue_date_unix,
            start_date_unix: n.start_date_unix,
            end_date_unix: n.end_date_unix,
            message: n.message,
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = NotamSnapshot {
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

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedNotam>,
    }

    #[async_trait]
    impl NotamFetcher for StaticFetcher {
        async fn fetch_notams(
            &self,
            _designators: &[&str],
        ) -> Result<Vec<FetchedNotam>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl NotamFetcher for FailingFetcher {
        async fn fetch_notams(
            &self,
            _designators: &[&str],
        ) -> Result<Vec<FetchedNotam>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn notam(number: &str, icao: &str, start: i64, end: i64) -> FetchedNotam {
        FetchedNotam {
            number: number.into(),
            icao_location: icao.into(),
            issue_date_unix: start - 3600,
            start_date_unix: start,
            end_date_unix: end,
            message: format!("{number} {icao} TWY CLSD"),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "aviation:active-notams:v1");
    }

    #[test]
    fn default_basket_includes_jfk_and_lhr() {
        let cfg = NotamConfig::default();
        assert!(cfg.designators.iter().any(|d| d == "KJFK"));
        assert!(cfg.designators.iter().any(|d| d == "EGLL"));
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_active_now_pre_computed() {
        let pool = open_in_memory().await.unwrap();
        let now_unix = pellucid_core::now_ms() / 1000;
        let fetcher = StaticFetcher {
            rows: vec![
                // Active right now.
                notam("A1/26", "KJFK", now_unix - 60, now_unix + 60),
                // Future-dated (not yet active).
                notam("A2/26", "KEWR", now_unix + 86_400, now_unix + 172_800),
                // Expired.
                notam("A3/26", "KLAX", now_unix - 172_800, now_unix - 86_400),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &NotamConfig::default())
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
        assert_eq!(rows[0].get("active_now").unwrap().as_bool(), Some(true));
        assert_eq!(rows[1].get("active_now").unwrap().as_bool(), Some(false));
        assert_eq!(rows[2].get("active_now").unwrap().as_bool(), Some(false));
    }

    #[tokio::test]
    async fn run_cycle_empty_upstream_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &NotamConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, AviationSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &NotamConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, AviationSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let now_unix = pellucid_core::now_ms() / 1000;
        let fetcher = StaticFetcher {
            rows: vec![notam("A1/26", "KJFK", now_unix, now_unix + 3600)],
        };
        let _ = run_cycle(&pool, &fetcher, &NotamConfig::default())
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
