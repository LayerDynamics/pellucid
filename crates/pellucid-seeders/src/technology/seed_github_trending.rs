//! seed_github_trending — FAST-tier roll-up of trending repos
//! from OSSInsight (the GitHub trending API was deprecated
//! in 2018; OSSInsight is the canonical free replacement).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::technology::TechnologySeederError;

/// Cache key — NEW FAST tier slot added by T3.8 expansion.
pub const CACHE_KEY: &str = "technology:github-trending:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "github-trending-ossinsight-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "technology-github";

/// Trending period mirror.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrendingWindow {
    /// Past 24 hours.
    Day,
    /// Past 7 days.
    Week,
    /// Past 30 days.
    Month,
}

impl TrendingWindow {
    /// Slug forwarded to the OSSInsight client.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Day => "past_24_hours",
            Self::Week => "past_week",
            Self::Month => "past_month",
        }
    }
}

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct GithubTrendingConfig {
    /// Trending window.
    pub window: TrendingWindow,
    /// Optional language filter.
    pub language: Option<String>,
    /// Top-N cap on retained repos.
    pub top_n: usize,
}

impl Default for GithubTrendingConfig {
    fn default() -> Self {
        Self {
            window: TrendingWindow::Day,
            language: None,
            top_n: 30,
        }
    }
}

/// One trending-repo row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepoRow {
    /// `owner/name`.
    pub repo_name: String,
    /// Star count.
    pub stars: i64,
    /// Fork count.
    pub forks: i64,
    /// Push count over the window.
    pub pushes: i64,
    /// Composite trending score.
    pub total_score: f64,
    /// Primary language.
    pub primary_language: String,
    /// Description.
    pub description: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GithubTrendingSnapshot {
    /// Top-N repos by trending score.
    pub rows: Vec<RepoRow>,
    /// Echo of the window slug.
    pub window: String,
    /// Echo of the language filter (empty when none).
    pub language: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled repo — mirrors `pellucid_streams::TrendingRepo`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedRepo {
    /// Repo name.
    pub repo_name: String,
    /// Stars.
    pub stars: i64,
    /// Forks.
    pub forks: i64,
    /// Pushes.
    pub pushes: i64,
    /// Score.
    pub total_score: f64,
    /// Language.
    pub primary_language: String,
    /// Description.
    pub description: String,
}

/// DI trait — wraps `pellucid_streams::OssInsightClient::fetch_trending`.
#[async_trait]
pub trait OssInsightFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch trending repos.
    async fn fetch_trending(
        &self,
        period_slug: &str,
        language: Option<&str>,
    ) -> Result<Vec<FetchedRepo>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`TechnologySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn OssInsightFetcher,
    config: &GithubTrendingConfig,
) -> Result<PublishOutcome, TechnologySeederError> {
    let fetched = fetcher
        .fetch_trending(config.window.slug(), config.language.as_deref())
        .await
        .map_err(|e| TechnologySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(TechnologySeederError::EmptyUpstream);
    }
    let mut rows: Vec<RepoRow> = fetched
        .into_iter()
        .map(|r| RepoRow {
            repo_name: r.repo_name,
            stars: r.stars,
            forks: r.forks,
            pushes: r.pushes,
            total_score: r.total_score,
            primary_language: r.primary_language,
            description: r.description,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.total_score
            .partial_cmp(&a.total_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(config.top_n);

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = GithubTrendingSnapshot {
        rows,
        window: config.window.slug().to_string(),
        language: config.language.clone().unwrap_or_default(),
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
        rows: Vec<FetchedRepo>,
    }

    #[async_trait]
    impl OssInsightFetcher for StaticFetcher {
        async fn fetch_trending(
            &self,
            _period_slug: &str,
            _language: Option<&str>,
        ) -> Result<Vec<FetchedRepo>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl OssInsightFetcher for FailingFetcher {
        async fn fetch_trending(
            &self,
            _period_slug: &str,
            _language: Option<&str>,
        ) -> Result<Vec<FetchedRepo>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn repo(name: &str, score: f64) -> FetchedRepo {
        FetchedRepo {
            repo_name: name.into(),
            stars: 1000,
            forks: 100,
            pushes: 10,
            total_score: score,
            primary_language: "Rust".into(),
            description: "x".into(),
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "technology:github-trending:v1");
    }

    #[test]
    fn window_slug_round_trips() {
        assert_eq!(TrendingWindow::Day.slug(), "past_24_hours");
        assert_eq!(TrendingWindow::Week.slug(), "past_week");
        assert_eq!(TrendingWindow::Month.slug(), "past_month");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_sorted_desc_truncated() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![repo("a", 0.50), repo("b", 0.92), repo("c", 0.81)],
        };
        let cfg = GithubTrendingConfig {
            top_n: 2,
            ..GithubTrendingConfig::default()
        };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("repo_name").unwrap().as_str().unwrap(), "b");
        assert_eq!(rows[1].get("repo_name").unwrap().as_str().unwrap(), "c");
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &GithubTrendingConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, TechnologySeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &GithubTrendingConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, TechnologySeederError::Upstream(_)));
    }
}
