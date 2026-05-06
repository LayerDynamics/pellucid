//! seed_scenario_library — SLOW-tier scenario library snapshot.
//! Production adapters wire to the Pellucid scenario-library
//! catalog (Postgres canonical source); tests inject deterministic
//! rows.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::prediction::PredictionSeederError;

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "prediction:scenario-library:v1";

/// 24 h TTL.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "scenario-library-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "prediction";

/// One scenario-library entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioRow {
    /// Scenario id.
    pub id: String,
    /// Headline.
    pub title: String,
    /// Domain — `geo`, `econ`, `cyber`, etc.
    pub domain: String,
    /// Crowd-implied probability 0..=1 at snapshot time.
    pub probability: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioLibrarySnapshot {
    /// Scenarios sorted descending by probability.
    pub rows: Vec<ScenarioRow>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched scenario.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedScenario {
    /// Scenario id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Domain.
    pub domain: String,
    /// Probability.
    pub probability: f64,
}

/// DI trait.
#[async_trait]
pub trait ScenarioLibraryFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the scenario library.
    async fn fetch_scenarios(
        &self,
    ) -> Result<Vec<FetchedScenario>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn ScenarioLibraryFetcher,
) -> Result<PublishOutcome, PredictionSeederError> {
    let fetched = fetcher
        .fetch_scenarios()
        .await
        .map_err(|e| PredictionSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(PredictionSeederError::EmptyUpstream);
    }
    let mut rows: Vec<ScenarioRow> = fetched
        .into_iter()
        .map(|s| ScenarioRow {
            id: s.id,
            title: s.title,
            domain: s.domain,
            probability: s.probability.clamp(0.0, 1.0),
        })
        .collect();
    rows.sort_by(|a, b| {
        b.probability
            .partial_cmp(&a.probability)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = ScenarioLibrarySnapshot {
        rows,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(86_400_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "prediction", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedScenario>,
    }

    #[async_trait]
    impl ScenarioLibraryFetcher for StaticFetcher {
        async fn fetch_scenarios(
            &self,
        ) -> Result<Vec<FetchedScenario>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    fn s(id: &str, p: f64) -> FetchedScenario {
        FetchedScenario {
            id: id.into(),
            title: format!("Scenario {id}"),
            domain: "geo".into(),
            probability: p,
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "prediction:scenario-library:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_descending() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![s("low", 0.12), s("hi", 0.81), s("mid", 0.42)],
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let ids: Vec<&str> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("id").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["hi", "mid", "low"]);
    }

    #[tokio::test]
    async fn run_cycle_clamps_probability() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![s("over", 1.5), s("under", -0.3)],
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let probs: Vec<f64> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("probability").unwrap().as_f64().unwrap())
            .collect();
        assert!(probs.iter().all(|p| (0.0..=1.0).contains(p)));
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, PredictionSeederError::EmptyUpstream));
    }
}
