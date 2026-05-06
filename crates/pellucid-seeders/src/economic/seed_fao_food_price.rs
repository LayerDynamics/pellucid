//! seed_fao_food_price — SLOW-tier FAO Food Price Index snapshot.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::economic::EconomicSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "economic:fao-food-price-index:v1";

/// SLOW-tier TTL — 24 h. FAO publishes monthly.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "fao-food-price-index-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "economic";

/// FAO sub-indices. The composite Food Price Index is the
/// weighted average of these.
pub const SUBINDEX_LABELS: &[&str] = &["meat", "dairy", "cereals", "oils", "sugar"];

/// One observation row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaoObservation {
    /// Period (`YYYY-MM`).
    pub period: String,
    /// Composite Food Price Index value.
    pub composite: f64,
    /// Per-sub-index values keyed by label.
    pub subindices: Vec<(String, f64)>,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaoSnapshot {
    /// Latest observation.
    pub latest: FaoObservation,
    /// Trailing observations sorted ascending by period.
    pub history: Vec<FaoObservation>,
    /// Year-over-year % change of the composite index.
    pub yoy_pct: f64,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled FAO row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedFaoObservation {
    /// Period.
    pub period: String,
    /// Composite value.
    pub composite: f64,
    /// Sub-index pairs (label, value).
    pub subindices: Vec<(String, f64)>,
}

/// DI trait.
#[async_trait]
pub trait FaoFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch trailing months of the FAO index.
    async fn fetch_history(
        &self,
        history_months: usize,
    ) -> Result<Vec<FetchedFaoObservation>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Compute YoY % from two composite values. Pure.
#[must_use]
pub fn yoy_pct(latest: f64, year_ago: f64) -> f64 {
    if year_ago.abs() < f64::EPSILON {
        return 0.0;
    }
    (latest - year_ago) / year_ago * 100.0
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn FaoFetcher,
    history_months: usize,
) -> Result<PublishOutcome, EconomicSeederError> {
    let fetched = fetcher
        .fetch_history(history_months)
        .await
        .map_err(|e| EconomicSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EconomicSeederError::EmptyUpstream);
    }
    let mut history: Vec<FaoObservation> = fetched
        .into_iter()
        .map(|o| FaoObservation {
            period: o.period,
            composite: o.composite,
            subindices: o.subindices,
        })
        .collect();
    history.sort_by(|a, b| a.period.cmp(&b.period));
    let Some(latest) = history.last().cloned() else {
        return Err(EconomicSeederError::EmptyUpstream);
    };
    // YoY: compare latest to the observation 12 entries back when present.
    let yoy = if history.len() >= 13 {
        let year_ago = &history[history.len() - 13];
        yoy_pct(latest.composite, year_ago.composite)
    } else {
        0.0
    };

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = FaoSnapshot {
        latest,
        history,
        yoy_pct: yoy,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(86_400_000),
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
        rows: Vec<FetchedFaoObservation>,
    }

    #[async_trait]
    impl FaoFetcher for StaticFetcher {
        async fn fetch_history(
            &self,
            _n: usize,
        ) -> Result<Vec<FetchedFaoObservation>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    fn obs(period: &str, composite: f64) -> FetchedFaoObservation {
        FetchedFaoObservation {
            period: period.into(),
            composite,
            subindices: SUBINDEX_LABELS
                .iter()
                .map(|s| ((*s).to_string(), 100.0))
                .collect(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "economic:fao-food-price-index:v1");
    }

    #[test]
    fn yoy_handles_zero_year_ago() {
        assert_eq!(yoy_pct(110.0, 0.0), 0.0);
        assert!((yoy_pct(110.0, 100.0) - 10.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn run_cycle_sorts_and_picks_latest_with_yoy_when_window_long_enough() {
        let pool = open_in_memory().await.unwrap();
        // 13 monthly obs so YoY can be computed.
        let mut rows: Vec<FetchedFaoObservation> = (0..13)
            .map(|i| obs(&format!("2025-{:02}", i + 1), 100.0 + i as f64))
            .collect();
        rows.reverse();
        let fetcher = StaticFetcher { rows };
        let _ = run_cycle(&pool, &fetcher, 13).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        assert_eq!(
            parsed
                .pointer("/data/latest/period")
                .and_then(serde_json::Value::as_str),
            Some("2025-13"),
        );
        // YoY: (112 - 100) / 100 * 100 = 12.0.
        assert!(
            (parsed
                .pointer("/data/yoy_pct")
                .and_then(serde_json::Value::as_f64)
                .unwrap()
                - 12.0)
                .abs()
                < 1e-6
        );
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, 12).await.unwrap_err();
        assert!(matches!(err, EconomicSeederError::EmptyUpstream));
    }
}
