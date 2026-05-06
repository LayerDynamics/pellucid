//! seed_big_mac_index — SLOW-tier Economist Big Mac Index
//! snapshot.
//!
//! The Economist publishes the Big Mac CSV at
//! https://github.com/TheEconomist/big-mac-data ; the production
//! adapter parses the CSV; tests inject deterministic rows.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::economic::EconomicSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "economic:big-mac-index:v1";

/// SLOW-tier TTL — 7 days. The Economist publishes biannually.
pub const TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "economist-big-mac-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "economic";

/// One country row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BigMacRow {
    /// ISO-3 country code.
    pub iso: String,
    /// Country name.
    pub country: String,
    /// Local-currency price.
    pub local_price: f64,
    /// Local currency code.
    pub currency: String,
    /// USD-equivalent price.
    pub usd_price: f64,
    /// Implied PPP exchange rate.
    pub ppp: f64,
    /// Actual exchange rate.
    pub fx_rate: f64,
    /// Over- (+) / under- (−) valuation vs USD in percent.
    pub valuation_pct: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BigMacSnapshot {
    /// Snapshot date as published by the Economist (`YYYY-MM-DD`).
    pub snapshot_date: String,
    /// Country rows sorted ascending by ISO.
    pub rows: Vec<BigMacRow>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedBigMacRow {
    /// ISO-3 code.
    pub iso: String,
    /// Country name.
    pub country: String,
    /// Local-currency price.
    pub local_price: f64,
    /// Currency code.
    pub currency: String,
    /// USD price.
    pub usd_price: f64,
    /// Implied PPP rate.
    pub ppp: f64,
    /// Actual FX rate.
    pub fx_rate: f64,
    /// Over/under valuation %.
    pub valuation_pct: f64,
}

/// DI trait.
#[async_trait]
pub trait BigMacFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the most recent published Big Mac snapshot. Returns
    /// the snapshot date + rows.
    async fn fetch_latest(
        &self,
    ) -> Result<(String, Vec<FetchedBigMacRow>), Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn BigMacFetcher,
) -> Result<PublishOutcome, EconomicSeederError> {
    let (date, fetched) = fetcher
        .fetch_latest()
        .await
        .map_err(|e| EconomicSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EconomicSeederError::EmptyUpstream);
    }
    let mut rows: Vec<BigMacRow> = fetched
        .into_iter()
        .map(|r| BigMacRow {
            iso: r.iso,
            country: r.country,
            local_price: r.local_price,
            currency: r.currency,
            usd_price: r.usd_price,
            ppp: r.ppp,
            fx_rate: r.fx_rate,
            valuation_pct: r.valuation_pct,
        })
        .collect();
    rows.sort_by(|a, b| a.iso.cmp(&b.iso));

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = BigMacSnapshot {
        snapshot_date: date,
        rows,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(604_800_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
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
        date: String,
        rows: Vec<FetchedBigMacRow>,
    }

    #[async_trait]
    impl BigMacFetcher for StaticFetcher {
        async fn fetch_latest(
            &self,
        ) -> Result<(String, Vec<FetchedBigMacRow>), Box<dyn std::error::Error + Send + Sync>>
        {
            Ok((self.date.clone(), self.rows.clone()))
        }
    }

    fn row(iso: &str, country: &str, valuation: f64) -> FetchedBigMacRow {
        FetchedBigMacRow {
            iso: iso.into(),
            country: country.into(),
            local_price: 5.0,
            currency: "USD".into(),
            usd_price: 5.0,
            ppp: 5.0,
            fx_rate: 1.0,
            valuation_pct: valuation,
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "economic:big-mac-index:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_rows() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            date: "2026-01-01".into(),
            rows: vec![
                row("USA", "United States", 0.0),
                row("CHE", "Switzerland", 30.0),
                row("CHN", "China", -40.0),
            ],
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
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
        assert_eq!(isos, vec!["CHE", "CHN", "USA"]);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            date: "2026-01-01".into(),
            rows: vec![],
        };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, EconomicSeederError::EmptyUpstream));
    }
}
