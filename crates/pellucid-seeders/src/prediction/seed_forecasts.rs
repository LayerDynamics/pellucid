//! seed_forecasts — FAST-tier snapshot of active Metaculus
//! crowd forecasts.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::prediction::PredictionSeederError;

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "forecast:now-cast:summary:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "forecasts-metaculus-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "prediction-forecasts";

/// Default cap on returned questions.
pub const DEFAULT_LIMIT: u32 = 30;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct ForecastsConfig {
    /// Max questions to fetch.
    pub limit: u32,
}

impl Default for ForecastsConfig {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
        }
    }
}

/// One forecast row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForecastRow {
    /// Metaculus id.
    pub id: i64,
    /// Question title.
    pub title: String,
    /// Permalink path (relative to metaculus.com).
    pub page_url: String,
    /// Resolution timestamp.
    pub resolve_time: String,
    /// Activity score.
    pub activity: f64,
    /// Community median (q2) in `[0, 1]`.
    pub community_median: f64,
    /// 25th percentile.
    pub community_q1: f64,
    /// 75th percentile.
    pub community_q3: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForecastsSnapshot {
    /// Forecasts ranked by activity desc.
    pub rows: Vec<ForecastRow>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled question — mirrors `pellucid_streams::MetaculusQuestion`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedQuestion {
    /// Id.
    pub id: i64,
    /// Title.
    pub title: String,
    /// Permalink.
    pub page_url: String,
    /// Status.
    pub status: String,
    /// Resolve time.
    pub resolve_time: String,
    /// Activity.
    pub activity: f64,
    /// Median.
    pub community_median: f64,
    /// q1.
    pub community_q1: f64,
    /// q3.
    pub community_q3: f64,
}

/// DI trait — wraps `pellucid_streams::MetaculusClient::fetch_active_questions`.
#[async_trait]
pub trait ForecastsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch active questions.
    async fn fetch_active_questions(
        &self,
        limit: u32,
    ) -> Result<Vec<FetchedQuestion>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`PredictionSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn ForecastsFetcher,
    config: &ForecastsConfig,
) -> Result<PublishOutcome, PredictionSeederError> {
    let fetched = fetcher
        .fetch_active_questions(config.limit)
        .await
        .map_err(|e| PredictionSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(PredictionSeederError::EmptyUpstream);
    }
    let rows: Vec<ForecastRow> = fetched
        .into_iter()
        .map(|q| ForecastRow {
            id: q.id,
            title: q.title,
            page_url: q.page_url,
            resolve_time: q.resolve_time,
            activity: q.activity,
            community_median: q.community_median,
            community_q1: q.community_q1,
            community_q3: q.community_q3,
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = ForecastsSnapshot {
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
    let outcome =
        atomic_publish(pool, "forecast", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        questions: Vec<FetchedQuestion>,
    }

    #[async_trait]
    impl ForecastsFetcher for StaticFetcher {
        async fn fetch_active_questions(
            &self,
            _limit: u32,
        ) -> Result<Vec<FetchedQuestion>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.questions.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl ForecastsFetcher for FailingFetcher {
        async fn fetch_active_questions(
            &self,
            _limit: u32,
        ) -> Result<Vec<FetchedQuestion>, Box<dyn std::error::Error + Send + Sync>>
        {
            Err("upstream down".into())
        }
    }

    fn question(id: i64, median: f64) -> FetchedQuestion {
        FetchedQuestion {
            id,
            title: format!("Q{id}"),
            page_url: format!("/questions/{id}/q-{id}/"),
            status: "open".into(),
            resolve_time: "2026-12-31T00:00:00Z".into(),
            activity: 10.0 + id as f64,
            community_median: median,
            community_q1: median - 0.10,
            community_q3: median + 0.10,
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "forecast:now-cast:summary:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            questions: vec![question(1, 0.45), question(2, 0.62)],
        };
        let outcome = run_cycle(&pool, &fetcher, &ForecastsConfig::default())
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
        assert_eq!(rows.len(), 2);
        assert!(
            (rows[0].get("community_median").unwrap().as_f64().unwrap() - 0.45).abs()
                < 1e-9
        );
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { questions: vec![] };
        let err = run_cycle(&pool, &fetcher, &ForecastsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, PredictionSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &ForecastsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, PredictionSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            questions: vec![question(1, 0.45)],
        };
        let _ = run_cycle(&pool, &fetcher, &ForecastsConfig::default())
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
