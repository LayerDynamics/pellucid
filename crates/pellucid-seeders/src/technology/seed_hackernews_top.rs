//! seed_hackernews_top — FAST-tier roll-up of HN top stories.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::technology::TechnologySeederError;

/// Cache key — NEW FAST tier slot added by T3.8 expansion.
pub const CACHE_KEY: &str = "technology:hn-top-stories:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "hn-top-stories-firebase-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "technology-hn";

/// Default cap on retained stories.
pub const DEFAULT_LIMIT: usize = 30;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct HackerNewsTopConfig {
    /// Max stories to fetch.
    pub limit: usize,
}

impl Default for HackerNewsTopConfig {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
        }
    }
}

/// One story row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoryRow {
    /// HN id.
    pub id: u64,
    /// Submitter.
    pub by: String,
    /// Title.
    pub title: String,
    /// External URL (empty for text posts).
    pub url: String,
    /// Score (upvotes).
    pub score: i64,
    /// Comment count.
    pub descendants: i64,
    /// Submitted timestamp (unix seconds).
    pub time_unix: i64,
    /// HN item type.
    pub item_type: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HackerNewsTopSnapshot {
    /// Stories — ordered by HN's own top-story ranking.
    pub rows: Vec<StoryRow>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled story — mirrors `pellucid_streams::HnStory`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedStory {
    /// Id.
    pub id: u64,
    /// By.
    pub by: String,
    /// Title.
    pub title: String,
    /// URL.
    pub url: String,
    /// Score.
    pub score: i64,
    /// Comments.
    pub descendants: i64,
    /// Time.
    pub time_unix: i64,
    /// Type.
    pub item_type: String,
}

/// DI trait — wraps `pellucid_streams::HackerNewsClient::fetch_top_stories`.
#[async_trait]
pub trait HackerNewsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the top-`limit` resolved stories.
    async fn fetch_top_stories(
        &self,
        limit: usize,
    ) -> Result<Vec<FetchedStory>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`TechnologySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn HackerNewsFetcher,
    config: &HackerNewsTopConfig,
) -> Result<PublishOutcome, TechnologySeederError> {
    let fetched = fetcher
        .fetch_top_stories(config.limit)
        .await
        .map_err(|e| TechnologySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(TechnologySeederError::EmptyUpstream);
    }
    let rows: Vec<StoryRow> = fetched
        .into_iter()
        .map(|s| StoryRow {
            id: s.id,
            by: s.by,
            title: s.title,
            url: s.url,
            score: s.score,
            descendants: s.descendants,
            time_unix: s.time_unix,
            item_type: s.item_type,
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = HackerNewsTopSnapshot {
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
    let outcome = atomic_publish(pool, "technology", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedStory>,
    }

    #[async_trait]
    impl HackerNewsFetcher for StaticFetcher {
        async fn fetch_top_stories(
            &self,
            _limit: usize,
        ) -> Result<Vec<FetchedStory>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl HackerNewsFetcher for FailingFetcher {
        async fn fetch_top_stories(
            &self,
            _limit: usize,
        ) -> Result<Vec<FetchedStory>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn story(id: u64, score: i64) -> FetchedStory {
        FetchedStory {
            id,
            by: "alice".into(),
            title: format!("Story {id}"),
            url: format!("https://example.com/{id}"),
            score,
            descendants: 42,
            time_unix: 1_714_060_800,
            item_type: "story".into(),
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "technology:hn-top-stories:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![story(1, 500), story(2, 300)],
        };
        let outcome = run_cycle(&pool, &fetcher, &HackerNewsTopConfig::default())
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
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &HackerNewsTopConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, TechnologySeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &HackerNewsTopConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, TechnologySeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![story(1, 500)],
        };
        let _ = run_cycle(&pool, &fetcher, &HackerNewsTopConfig::default())
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
