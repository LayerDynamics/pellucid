//! seed_active_deployments — FAST-tier roll-up of news
//! coverage matching the GDELT military-deployment theme set.
//!
//! Real-time per-unit deployment data is paywalled (Janes /
//! IHS / USNI Fleet Tracker subscription) — there is no free
//! public REST endpoint for current ship/aircraft positions
//! by command. The closest free signal is news coverage of
//! deployment events via GDELT. The seeder publishes the
//! resulting article roll-up + a `source_note` documenting
//! the constraint.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::conflict::seed_gdelt_intel::{FetchedGdeltArticle, GdeltFetcher};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::military::MilitarySeederError;

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "military:active-deployments:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "active-deployments-gdelt-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "military-deployments";

/// Default GDELT query — military deployment / movement /
/// exercise themes.
pub const DEFAULT_QUERY: &str =
    "(theme:MILITARY_DEPLOYMENT OR theme:MILITARY_EXERCISE OR theme:NAVAL_DEPLOYMENT OR (theme:MILITARY AND (theme:MOVEMENT OR theme:DEPLOYMENT)))";

/// Default lookback.
pub const DEFAULT_TIMESPAN: &str = "24h";

/// Default record cap.
pub const DEFAULT_MAX_RECORDS: u32 = 75;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct ActiveDeploymentsConfig {
    /// GDELT query.
    pub query: String,
    /// Lookback window.
    pub timespan: String,
    /// Max records.
    pub max_records: u32,
}

impl Default for ActiveDeploymentsConfig {
    fn default() -> Self {
        Self {
            query: DEFAULT_QUERY.to_string(),
            timespan: DEFAULT_TIMESPAN.to_string(),
            max_records: DEFAULT_MAX_RECORDS,
        }
    }
}

/// One article row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeploymentArticleRow {
    /// URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// Seen-date timestamp.
    pub seen_date: String,
    /// Source domain.
    pub domain: String,
    /// Source country.
    pub source_country: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveDeploymentsSnapshot {
    /// Articles in upstream order.
    pub rows: Vec<DeploymentArticleRow>,
    /// Echo of the query.
    pub query: String,
    /// Documented proxy note.
    pub source_note: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Run one cycle.
///
/// # Errors
/// See [`MilitarySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn GdeltFetcher,
    config: &ActiveDeploymentsConfig,
) -> Result<PublishOutcome, MilitarySeederError> {
    let fetched = fetcher
        .search_articles(&config.query, &config.timespan, config.max_records)
        .await
        .map_err(|e| MilitarySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(MilitarySeederError::EmptyUpstream);
    }
    let rows: Vec<DeploymentArticleRow> = fetched.into_iter().map(map_row).collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = ActiveDeploymentsSnapshot {
        rows,
        query: config.query.clone(),
        source_note: "Article-level deployment coverage via GDELT. \
            Per-unit position data (Jane's IHS, USNI Fleet Tracker) \
            requires a paid subscription; no free public REST API \
            publishes current ship/aircraft positions by command."
            .into(),
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
        atomic_publish(pool, "military", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn map_row(a: FetchedGdeltArticle) -> DeploymentArticleRow {
    DeploymentArticleRow {
        url: a.url,
        title: a.title,
        seen_date: a.seen_date,
        domain: a.domain,
        source_country: a.source_country,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedGdeltArticle>,
        last_query: std::sync::Mutex<String>,
    }

    #[async_trait]
    impl GdeltFetcher for StaticFetcher {
        async fn search_articles(
            &self,
            query: &str,
            _timespan: &str,
            _max_records: u32,
        ) -> Result<Vec<FetchedGdeltArticle>, Box<dyn std::error::Error + Send + Sync>>
        {
            *self.last_query.lock().unwrap() = query.to_string();
            Ok(self.rows.clone())
        }
    }

    fn article(url: &str) -> FetchedGdeltArticle {
        FetchedGdeltArticle {
            url: url.into(),
            title: format!("Deployment — {url}"),
            seen_date: "20260504T120000Z".into(),
            social_image: String::new(),
            domain: "navalnews.com".into(),
            language: "English".into(),
            source_country: "US".into(),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "military:active-deployments:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_source_note() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![article("https://a.com"), article("https://b.com")],
            last_query: std::sync::Mutex::new(String::new()),
        };
        let _ = run_cycle(&pool, &fetcher, &ActiveDeploymentsConfig::default())
            .await
            .unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let note = parsed
            .pointer("/data/source_note")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(note.contains("paid subscription"));
        assert!(note.contains("GDELT"));
    }

    #[tokio::test]
    async fn run_cycle_uses_deployment_query() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![article("https://a.com")],
            last_query: std::sync::Mutex::new(String::new()),
        };
        let _ = run_cycle(&pool, &fetcher, &ActiveDeploymentsConfig::default())
            .await
            .unwrap();
        let q = fetcher.last_query.lock().unwrap().clone();
        assert!(q.contains("MILITARY"));
        assert!(q.contains("DEPLOYMENT"));
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![],
            last_query: std::sync::Mutex::new(String::new()),
        };
        let err = run_cycle(&pool, &fetcher, &ActiveDeploymentsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MilitarySeederError::EmptyUpstream));
    }
}
