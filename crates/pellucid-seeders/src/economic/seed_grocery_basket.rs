//! seed_grocery_basket — SLOW-tier per-country grocery-basket
//! price snapshot. Production adapters wire to OECD / Eurostat /
//! BLS APIs; tests inject deterministic rows.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::economic::EconomicSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "consumer-prices:grocery-basket:v1";

/// SLOW-tier TTL — 24 h. Grocery price indexes update monthly
/// upstream; 24 h refresh ensures every boot picks up same-day
/// republishings.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "grocery-basket-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "consumer-prices";

/// One per-country basket row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroceryBasketRow {
    /// ISO-3 country code.
    pub iso: String,
    /// Country name.
    pub country: String,
    /// USD-equivalent basket cost.
    pub basket_usd: f64,
    /// YoY % change of the country basket.
    pub basket_yoy_pct: f64,
    /// Reference period (e.g. `2026-04`).
    pub period: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroceryBasketSnapshot {
    /// Country rows sorted ascending by ISO.
    pub rows: Vec<GroceryBasketRow>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedGroceryRow {
    /// ISO-3 code.
    pub iso: String,
    /// Country name.
    pub country: String,
    /// USD basket cost.
    pub basket_usd: f64,
    /// YoY %.
    pub basket_yoy_pct: f64,
    /// Period.
    pub period: String,
}

/// DI trait.
#[async_trait]
pub trait GroceryBasketFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest country basket rows.
    async fn fetch_baskets(
        &self,
    ) -> Result<Vec<FetchedGroceryRow>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn GroceryBasketFetcher,
) -> Result<PublishOutcome, EconomicSeederError> {
    let fetched = fetcher
        .fetch_baskets()
        .await
        .map_err(|e| EconomicSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EconomicSeederError::EmptyUpstream);
    }
    let mut rows: Vec<GroceryBasketRow> = fetched
        .into_iter()
        .map(|r| GroceryBasketRow {
            iso: r.iso,
            country: r.country,
            basket_usd: r.basket_usd,
            basket_yoy_pct: r.basket_yoy_pct,
            period: r.period,
        })
        .collect();
    rows.sort_by(|a, b| a.iso.cmp(&b.iso));

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = GroceryBasketSnapshot {
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
    let outcome = atomic_publish(pool, "consumer-prices", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedGroceryRow>,
    }

    #[async_trait]
    impl GroceryBasketFetcher for StaticFetcher {
        async fn fetch_baskets(
            &self,
        ) -> Result<Vec<FetchedGroceryRow>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    fn row(iso: &str, country: &str, usd: f64, yoy: f64) -> FetchedGroceryRow {
        FetchedGroceryRow {
            iso: iso.into(),
            country: country.into(),
            basket_usd: usd,
            basket_yoy_pct: yoy,
            period: "2026-04".into(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "consumer-prices:grocery-basket:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_rows() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                row("USA", "United States", 120.0, 3.2),
                row("DEU", "Germany", 110.0, 2.1),
                row("JPN", "Japan", 95.0, 1.5),
            ],
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) = sqlx::query_as(
            "SELECT payload FROM kv_envelope WHERE cache_key = ?",
        )
        .bind(CACHE_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let isos: Vec<&str> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("iso").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(isos, vec!["DEU", "JPN", "USA"]);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, EconomicSeederError::EmptyUpstream));
    }
}
