//! seed_hormuz_transits — SLOW-tier Strait of Hormuz vessel-transit
//! snapshot. Production adapters wire to the EIA "World Oil Transit
//! Chokepoints" feed plus the UNCTAD Liner Shipping Connectivity
//! breakdown for tanker counts; tests inject deterministic rows.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::energy::EnergySeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "energy:hormuz-transits:current:v1";

/// 12 h refresh — chokepoint flow data updates daily upstream.
pub const TTL: Duration = Duration::from_secs(12 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "hormuz-transits-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "energy";

/// Per-product transit row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HormuzRow {
    /// Product class — e.g. `crude`, `lng`, `refined`.
    pub product: String,
    /// Average daily million-barrels-equivalent through Hormuz.
    pub mb_per_day: f64,
    /// Share of total chokepoint throughput as %.
    pub share_pct: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HormuzSnapshot {
    /// Rows sorted by descending mb/day.
    pub rows: Vec<HormuzRow>,
    /// Total mb/day through the strait.
    pub total_mb_per_day: f64,
    /// Reference period (e.g. `2026-04`).
    pub period: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedHormuzRow {
    /// Product class.
    pub product: String,
    /// mb/day.
    pub mb_per_day: f64,
}

/// DI trait.
#[async_trait]
pub trait HormuzTransitsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest chokepoint rows + period stamp.
    async fn fetch_transits(
        &self,
    ) -> Result<(Vec<FetchedHormuzRow>, String), Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn HormuzTransitsFetcher,
) -> Result<PublishOutcome, EnergySeederError> {
    let (fetched, period) = fetcher
        .fetch_transits()
        .await
        .map_err(|e| EnergySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EnergySeederError::EmptyUpstream);
    }
    let total: f64 = fetched.iter().map(|r| r.mb_per_day).sum();
    let mut rows: Vec<HormuzRow> = fetched
        .into_iter()
        .map(|r| HormuzRow {
            product: r.product,
            share_pct: if total > 0.0 { (r.mb_per_day / total) * 100.0 } else { 0.0 },
            mb_per_day: r.mb_per_day,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.mb_per_day
            .partial_cmp(&a.mb_per_day)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = HormuzSnapshot {
        rows,
        total_mb_per_day: total,
        period,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(43_200_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "energy", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedHormuzRow>,
        period: String,
    }

    #[async_trait]
    impl HormuzTransitsFetcher for StaticFetcher {
        async fn fetch_transits(
            &self,
        ) -> Result<(Vec<FetchedHormuzRow>, String), Box<dyn std::error::Error + Send + Sync>>
        {
            Ok((self.rows.clone(), self.period.clone()))
        }
    }

    fn row(product: &str, mb: f64) -> FetchedHormuzRow {
        FetchedHormuzRow {
            product: product.into(),
            mb_per_day: mb,
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "energy:hormuz-transits:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_with_shares() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                row("crude", 14.0),
                row("lng", 4.0),
                row("refined", 2.0),
            ],
            period: "2026-04".into(),
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let total = parsed.pointer("/data/total_mb_per_day").unwrap().as_f64().unwrap();
        assert!((total - 20.0).abs() < 0.001);
        let products: Vec<&str> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("product").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(products, vec!["crude", "lng", "refined"]);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![],
            period: "2026-04".into(),
        };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, EnergySeederError::EmptyUpstream));
    }
}
