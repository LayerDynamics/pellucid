//! seed_gdelt_intel — FAST-tier snapshot of recent GDELT
//! news articles matching the global "incident" theme set.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::conflict::ConflictSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "conflict:incident-feed:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "gdelt-doc-incident-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "conflict-gdelt";

/// Default GDELT DOC query for the global incident feed.
/// Targets violent-events themes per the GDELT Global Knowledge
/// Graph 2.0 vocabulary.
pub const DEFAULT_QUERY: &str = "(theme:KILL OR theme:WOUND OR theme:ARMEDCONFLICT OR theme:TERROR)";

/// Default lookback window — 24 hours.
pub const DEFAULT_TIMESPAN: &str = "24h";

/// Default max records per cycle.
pub const DEFAULT_MAX_RECORDS: u32 = 75;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct GdeltIntelConfig {
    /// Query string passed verbatim to GDELT.
    pub query: String,
    /// `24h` / `7d` / `1mo` etc.
    pub timespan: String,
    /// Cap on returned records.
    pub max_records: u32,
}

impl Default for GdeltIntelConfig {
    fn default() -> Self {
        Self {
            query: DEFAULT_QUERY.to_string(),
            timespan: DEFAULT_TIMESPAN.to_string(),
            max_records: DEFAULT_MAX_RECORDS,
        }
    }
}

/// One article row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GdeltArticleRow {
    /// Article URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// `YYYYMMDDTHHMMSSZ` timestamp.
    pub seen_date: String,
    /// Social-share image URL.
    pub social_image: String,
    /// Source domain.
    pub domain: String,
    /// Reported language.
    pub language: String,
    /// Source country.
    pub source_country: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GdeltIntelSnapshot {
    /// Articles in upstream order (newest-first per query sort).
    pub rows: Vec<GdeltArticleRow>,
    /// Echo of the query string.
    pub query: String,
    /// Echo of the timespan.
    pub timespan: String,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled article — mirrors `pellucid_streams::GdeltArticle`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedGdeltArticle {
    /// URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// Seen-date timestamp.
    pub seen_date: String,
    /// Social image URL.
    pub social_image: String,
    /// Source domain.
    pub domain: String,
    /// Language.
    pub language: String,
    /// Source country.
    pub source_country: String,
}

/// DI trait — wraps `pellucid_streams::GdeltClient::search_articles`.
#[async_trait]
pub trait GdeltFetcher: Send + Sync + std::fmt::Debug {
    /// Search GDELT DOC.
    async fn search_articles(
        &self,
        query: &str,
        timespan: &str,
        max_records: u32,
    ) -> Result<Vec<FetchedGdeltArticle>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`ConflictSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn GdeltFetcher,
    config: &GdeltIntelConfig,
) -> Result<PublishOutcome, ConflictSeederError> {
    let fetched = fetcher
        .search_articles(&config.query, &config.timespan, config.max_records)
        .await
        .map_err(|e| ConflictSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(ConflictSeederError::EmptyUpstream);
    }
    let rows: Vec<GdeltArticleRow> = fetched
        .into_iter()
        .map(|a| GdeltArticleRow {
            url: a.url,
            title: a.title,
            seen_date: a.seen_date,
            social_image: a.social_image,
            domain: a.domain,
            language: a.language,
            source_country: a.source_country,
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = GdeltIntelSnapshot {
        rows,
        query: config.query.clone(),
        timespan: config.timespan.clone(),
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
        atomic_publish(pool, "conflict", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
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

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl GdeltFetcher for FailingFetcher {
        async fn search_articles(
            &self,
            _query: &str,
            _timespan: &str,
            _max_records: u32,
        ) -> Result<Vec<FetchedGdeltArticle>, Box<dyn std::error::Error + Send + Sync>>
        {
            Err("upstream down".into())
        }
    }

    fn article(url: &str, country: &str) -> FetchedGdeltArticle {
        FetchedGdeltArticle {
            url: url.into(),
            title: format!("Title — {url}"),
            seen_date: "20260504T120000Z".into(),
            social_image: String::new(),
            domain: "example.com".into(),
            language: "English".into(),
            source_country: country.into(),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "conflict:incident-feed:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_passing_query_through() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![article("https://a.com", "Iran"), article("https://b.com", "Iraq")],
            last_query: std::sync::Mutex::new(String::new()),
        };
        let outcome = run_cycle(&pool, &fetcher, &GdeltIntelConfig::default())
            .await
            .unwrap();
        assert!(outcome.bytes_written > 0);
        assert_eq!(*fetcher.last_query.lock().unwrap(), DEFAULT_QUERY);
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

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![],
            last_query: std::sync::Mutex::new(String::new()),
        };
        let err = run_cycle(&pool, &fetcher, &GdeltIntelConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ConflictSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &GdeltIntelConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ConflictSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![article("https://a.com", "Iran")],
            last_query: std::sync::Mutex::new(String::new()),
        };
        let _ = run_cycle(&pool, &fetcher, &GdeltIntelConfig::default())
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
