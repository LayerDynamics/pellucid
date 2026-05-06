//! seed_financial_stress — SLOW-tier St. Louis Fed Financial
//! Stress Index (FRED `STLFSI4`).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::economic::EconomicSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "economic:financial-stress:v1";

/// SLOW-tier TTL — 12 h.
pub const TTL: Duration = Duration::from_secs(12 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "fred-stlfsi4-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "economic";

/// FRED series id.
pub const SERIES_CODE: &str = "STLFSI4";

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct FinancialStressConfig {
    /// FRED series code.
    pub series_code: String,
    /// How many trailing observations to keep in the snapshot.
    pub history_observations: usize,
}

impl Default for FinancialStressConfig {
    fn default() -> Self {
        Self {
            series_code: SERIES_CODE.to_string(),
            history_observations: 52,
        }
    }
}

/// One observation in the snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StressObservation {
    /// Observation date (`YYYY-MM-DD`).
    pub date: String,
    /// Index value.
    pub value: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FinancialStressSnapshot {
    /// Echo of the FRED series code.
    pub series_code: String,
    /// Latest observation (also the last entry of `history`).
    pub latest: StressObservation,
    /// Previous observation, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior: Option<StressObservation>,
    /// Trailing observations sorted ascending by date.
    pub history: Vec<StressObservation>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled FRED observation row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedStressObservation {
    /// Observation date.
    pub date: String,
    /// Index value.
    pub value: f64,
}

/// DI trait — wraps `pellucid_streams::FredClient::fetch_observations`.
#[async_trait]
pub trait FinancialStressFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch trailing observations for `series_code`. Returns
    /// rows in any order; the seeder sorts ascending by date.
    async fn fetch_observations(
        &self,
        series_code: &str,
        history_observations: usize,
    ) -> Result<Vec<FetchedStressObservation>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn FinancialStressFetcher,
    config: &FinancialStressConfig,
) -> Result<PublishOutcome, EconomicSeederError> {
    let fetched = fetcher
        .fetch_observations(&config.series_code, config.history_observations)
        .await
        .map_err(|e| EconomicSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EconomicSeederError::EmptyUpstream);
    }
    let mut history: Vec<StressObservation> = fetched
        .into_iter()
        .map(|o| StressObservation {
            date: o.date,
            value: o.value,
        })
        .collect();
    history.sort_by(|a, b| a.date.cmp(&b.date));
    let latest = history.last().cloned().expect("non-empty after check");
    let prior = if history.len() >= 2 {
        history.get(history.len() - 2).cloned()
    } else {
        None
    };

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = FinancialStressSnapshot {
        series_code: config.series_code.clone(),
        latest,
        prior,
        history,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(43_200_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.history.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "economic", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedStressObservation>,
    }

    #[async_trait]
    impl FinancialStressFetcher for StaticFetcher {
        async fn fetch_observations(
            &self,
            _series: &str,
            _n: usize,
        ) -> Result<Vec<FetchedStressObservation>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    fn obs(date: &str, v: f64) -> FetchedStressObservation {
        FetchedStressObservation {
            date: date.into(),
            value: v,
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "economic:financial-stress:v1");
    }

    #[tokio::test]
    async fn run_cycle_sorts_by_date_and_picks_latest_plus_prior() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                obs("2026-04-29", 0.5),
                obs("2026-04-15", 0.3),
                obs("2026-04-22", 0.4),
            ],
        };
        let _ = run_cycle(&pool, &fetcher, &FinancialStressConfig::default())
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as(
            "SELECT payload FROM kv_envelope WHERE cache_key = ?",
        )
        .bind(CACHE_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        assert_eq!(
            parsed.pointer("/data/latest/date").and_then(serde_json::Value::as_str),
            Some("2026-04-29"),
        );
        assert_eq!(
            parsed.pointer("/data/prior/date").and_then(serde_json::Value::as_str),
            Some("2026-04-22"),
        );
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &FinancialStressConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, EconomicSeederError::EmptyUpstream));
    }
}
