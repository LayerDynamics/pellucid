//! seed_polymarket — FAST-tier snapshot of active Polymarket
//! prediction markets ranked by 24h volume.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::prediction::PredictionSeederError;

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "prediction:scenario-state:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "polymarket-active-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "prediction-polymarket";

/// Default cap on returned markets.
pub const DEFAULT_LIMIT: u32 = 50;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct PolymarketConfig {
    /// Max markets to fetch.
    pub limit: u32,
}

impl Default for PolymarketConfig {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
        }
    }
}

/// One market row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketRow {
    /// Polymarket id.
    pub id: String,
    /// Question text.
    pub question: String,
    /// Stable URL slug.
    pub slug: String,
    /// Outcome labels.
    pub outcomes: Vec<String>,
    /// Outcome prices (aligned with `outcomes`).
    pub outcome_prices: Vec<f64>,
    /// 24-hour volume (USDC).
    pub volume_24hr: f64,
    /// Lifetime volume (USDC).
    pub volume: f64,
    /// AMM liquidity (USDC).
    pub liquidity: f64,
    /// Resolution date (ISO-8601).
    pub end_date: String,
    /// Topic category.
    pub category: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolymarketSnapshot {
    /// Markets ranked by 24h volume desc.
    pub rows: Vec<MarketRow>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled market — mirrors `pellucid_streams::PredictionMarket`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedMarket {
    /// Id.
    pub id: String,
    /// Question.
    pub question: String,
    /// Slug.
    pub slug: String,
    /// Outcomes.
    pub outcomes: Vec<String>,
    /// Prices.
    pub outcome_prices: Vec<f64>,
    /// Lifetime volume.
    pub volume: f64,
    /// 24h volume.
    pub volume_24hr: f64,
    /// Liquidity.
    pub liquidity: f64,
    /// End date.
    pub end_date: String,
    /// Category.
    pub category: String,
}

/// DI trait — wraps `pellucid_streams::PolymarketClient::fetch_active_markets`.
#[async_trait]
pub trait PolymarketFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch active markets.
    async fn fetch_active_markets(
        &self,
        limit: u32,
    ) -> Result<Vec<FetchedMarket>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`PredictionSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn PolymarketFetcher,
    config: &PolymarketConfig,
) -> Result<PublishOutcome, PredictionSeederError> {
    let fetched = fetcher
        .fetch_active_markets(config.limit)
        .await
        .map_err(|e| PredictionSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(PredictionSeederError::EmptyUpstream);
    }
    let rows: Vec<MarketRow> = fetched
        .into_iter()
        .map(|m| MarketRow {
            id: m.id,
            question: m.question,
            slug: m.slug,
            outcomes: m.outcomes,
            outcome_prices: m.outcome_prices,
            volume_24hr: m.volume_24hr,
            volume: m.volume,
            liquidity: m.liquidity,
            end_date: m.end_date,
            category: m.category,
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = PolymarketSnapshot {
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
        markets: Vec<FetchedMarket>,
    }

    #[async_trait]
    impl PolymarketFetcher for StaticFetcher {
        async fn fetch_active_markets(
            &self,
            _limit: u32,
        ) -> Result<Vec<FetchedMarket>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.markets.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl PolymarketFetcher for FailingFetcher {
        async fn fetch_active_markets(
            &self,
            _limit: u32,
        ) -> Result<Vec<FetchedMarket>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn market(id: &str, vol_24h: f64) -> FetchedMarket {
        FetchedMarket {
            id: id.into(),
            question: format!("Question {id}"),
            slug: format!("question-{id}"),
            outcomes: vec!["Yes".into(), "No".into()],
            outcome_prices: vec![0.62, 0.38],
            volume: 1_000_000.0,
            volume_24hr: vol_24h,
            liquidity: 100_000.0,
            end_date: "2026-12-31T00:00:00Z".into(),
            category: "Politics".into(),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "prediction:scenario-state:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            markets: vec![market("1", 12_345.0), market("2", 6_789.0)],
        };
        let outcome = run_cycle(&pool, &fetcher, &PolymarketConfig::default())
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
        assert_eq!(rows[0].get("id").unwrap().as_str().unwrap(), "1");
        let prices = rows[0].get("outcome_prices").unwrap().as_array().unwrap();
        assert!((prices[0].as_f64().unwrap() - 0.62).abs() < 1e-9);
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { markets: vec![] };
        let err = run_cycle(&pool, &fetcher, &PolymarketConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, PredictionSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &PolymarketConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, PredictionSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            markets: vec![market("1", 12_345.0)],
        };
        let _ = run_cycle(&pool, &fetcher, &PolymarketConfig::default())
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
