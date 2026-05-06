//! seed_extended_forecast — SLOW-tier weekly extended-forecast
//! snapshot. Production adapters wire to the Metaculus + Good
//! Judgment Open + Polymarket combined extended-horizon feed;
//! tests inject deterministic forecast rows.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::prediction::PredictionSeederError;

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "forecast:extended:weekly:v1";

/// 24 h TTL.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "extended-forecast-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "forecast";

/// One extended-forecast row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtendedForecastRow {
    /// Question id.
    pub id: String,
    /// Question text.
    pub question: String,
    /// Crowd probability 0..=1.
    pub probability: f64,
    /// Resolution horizon — `1w`, `1m`, `1q`, `1y`.
    pub horizon: String,
    /// 7-day delta in probability points (signed).
    pub delta_7d: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtendedForecastSnapshot {
    /// Forecasts sorted by descending |delta_7d|.
    pub rows: Vec<ExtendedForecastRow>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched forecast.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedExtendedForecast {
    /// Question id.
    pub id: String,
    /// Question.
    pub question: String,
    /// Probability.
    pub probability: f64,
    /// Horizon.
    pub horizon: String,
    /// 7-day delta.
    pub delta_7d: f64,
}

/// DI trait.
#[async_trait]
pub trait ExtendedForecastFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch extended forecasts.
    async fn fetch_forecasts(
        &self,
    ) -> Result<Vec<FetchedExtendedForecast>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn ExtendedForecastFetcher,
) -> Result<PublishOutcome, PredictionSeederError> {
    let fetched = fetcher
        .fetch_forecasts()
        .await
        .map_err(|e| PredictionSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(PredictionSeederError::EmptyUpstream);
    }
    let mut rows: Vec<ExtendedForecastRow> = fetched
        .into_iter()
        .map(|f| ExtendedForecastRow {
            id: f.id,
            question: f.question,
            probability: f.probability.clamp(0.0, 1.0),
            horizon: f.horizon,
            delta_7d: f.delta_7d,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.delta_7d
            .abs()
            .partial_cmp(&a.delta_7d.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = ExtendedForecastSnapshot {
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
    let outcome = atomic_publish(pool, "forecast", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedExtendedForecast>,
    }

    #[async_trait]
    impl ExtendedForecastFetcher for StaticFetcher {
        async fn fetch_forecasts(
            &self,
        ) -> Result<Vec<FetchedExtendedForecast>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    fn f(id: &str, p: f64, d: f64) -> FetchedExtendedForecast {
        FetchedExtendedForecast {
            id: id.into(),
            question: format!("Q {id}"),
            probability: p,
            horizon: "1m".into(),
            delta_7d: d,
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "forecast:extended:weekly:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_by_abs_delta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![f("a", 0.5, 0.02), f("b", 0.4, -0.08), f("c", 0.7, 0.05)],
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
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
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, PredictionSeederError::EmptyUpstream));
    }
}
