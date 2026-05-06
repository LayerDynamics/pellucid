//! seed_renewable_mix — SLOW-tier global renewable-energy mix
//! snapshot. Production adapters wire to the IRENA "Renewable
//! Capacity Statistics" + Ember "Global Electricity Review" feeds;
//! tests inject deterministic rows.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::energy::EnergySeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "energy:renewable-mix:current:v1";

/// 24 h refresh — capacity datasets republish quarterly upstream.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "renewable-mix-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "energy";

/// One source-mix row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenewableSourceRow {
    /// Source class — e.g. `solar`, `wind`, `hydro`, `geothermal`,
    /// `bioenergy`.
    pub source: String,
    /// Installed capacity in gigawatts.
    pub capacity_gw: f64,
    /// YoY % change of capacity.
    pub yoy_pct: f64,
    /// Share of total renewable capacity as %.
    pub share_pct: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenewableMixSnapshot {
    /// Rows sorted by descending capacity.
    pub rows: Vec<RenewableSourceRow>,
    /// Total renewable capacity in GW.
    pub total_capacity_gw: f64,
    /// Reference period (e.g. `2026-Q1`).
    pub period: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedRenewableRow {
    /// Source class.
    pub source: String,
    /// Capacity in GW.
    pub capacity_gw: f64,
    /// YoY %.
    pub yoy_pct: f64,
}

/// DI trait.
#[async_trait]
pub trait RenewableMixFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest source-mix rows + period stamp.
    async fn fetch_mix(
        &self,
    ) -> Result<(Vec<FetchedRenewableRow>, String), Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn RenewableMixFetcher,
) -> Result<PublishOutcome, EnergySeederError> {
    let (fetched, period) = fetcher
        .fetch_mix()
        .await
        .map_err(|e| EnergySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EnergySeederError::EmptyUpstream);
    }
    let total: f64 = fetched.iter().map(|r| r.capacity_gw).sum();
    let mut rows: Vec<RenewableSourceRow> = fetched
        .into_iter()
        .map(|r| RenewableSourceRow {
            share_pct: if total > 0.0 {
                (r.capacity_gw / total) * 100.0
            } else {
                0.0
            },
            source: r.source,
            capacity_gw: r.capacity_gw,
            yoy_pct: r.yoy_pct,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.capacity_gw
            .partial_cmp(&a.capacity_gw)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = RenewableMixSnapshot {
        rows,
        total_capacity_gw: total,
        period,
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
        rows: Vec<FetchedRenewableRow>,
        period: String,
    }

    #[async_trait]
    impl RenewableMixFetcher for StaticFetcher {
        async fn fetch_mix(
            &self,
        ) -> Result<(Vec<FetchedRenewableRow>, String), Box<dyn std::error::Error + Send + Sync>>
        {
            Ok((self.rows.clone(), self.period.clone()))
        }
    }

    fn row(source: &str, gw: f64, yoy: f64) -> FetchedRenewableRow {
        FetchedRenewableRow {
            source: source.into(),
            capacity_gw: gw,
            yoy_pct: yoy,
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "energy:renewable-mix:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_with_shares() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                row("hydro", 1300.0, 1.0),
                row("solar", 1500.0, 22.0),
                row("wind", 1000.0, 12.0),
            ],
            period: "2026-Q1".into(),
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let sources: Vec<&str> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("source").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(sources, vec!["solar", "hydro", "wind"]);
        let total = parsed
            .pointer("/data/total_capacity_gw")
            .unwrap()
            .as_f64()
            .unwrap();
        assert!((total - 3800.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![],
            period: "2026-Q1".into(),
        };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, EnergySeederError::EmptyUpstream));
    }
}
