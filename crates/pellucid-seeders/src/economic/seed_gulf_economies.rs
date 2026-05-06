//! seed_gulf_economies — SLOW-tier per-country economic
//! snapshot for the GCC + Iran (the markets domain treats
//! these as a regional cluster). Production adapters compose
//! IMF / World Bank / FRED rows; tests inject deterministic data.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::economic::EconomicSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "economic:gulf-economies:v1";

/// SLOW-tier TTL — 24 h.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "gulf-economies-composite-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "economic";

/// Default ISO-3 country basket (GCC + Iran).
pub const DEFAULT_COUNTRIES: &[&str] = &[
    "SAU", "ARE", "QAT", "KWT", "BHR", "OMN", "IRN",
];

/// One country row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GulfCountryRow {
    /// ISO-3 country code.
    pub iso: String,
    /// Country display name.
    pub country: String,
    /// GDP latest (USD billion).
    pub gdp_usd_billion: f64,
    /// GDP YoY %.
    pub gdp_yoy_pct: f64,
    /// Inflation YoY %.
    pub inflation_yoy_pct: f64,
    /// Unemployment %.
    pub unemployment_pct: f64,
    /// Policy rate %.
    pub policy_rate_pct: f64,
    /// Reference period (e.g. `2026-Q1`).
    pub period: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GulfSnapshot {
    /// Country rows sorted ascending by ISO.
    pub rows: Vec<GulfCountryRow>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedGulfRow {
    /// ISO-3.
    pub iso: String,
    /// Country.
    pub country: String,
    /// GDP $B.
    pub gdp_usd_billion: f64,
    /// GDP YoY.
    pub gdp_yoy_pct: f64,
    /// Inflation YoY.
    pub inflation_yoy_pct: f64,
    /// Unemployment.
    pub unemployment_pct: f64,
    /// Policy rate.
    pub policy_rate_pct: f64,
    /// Period.
    pub period: String,
}

/// DI trait.
#[async_trait]
pub trait GulfEconomiesFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest per-country rows for the basket.
    async fn fetch_basket(
        &self,
        isos: &[&str],
    ) -> Result<Vec<FetchedGulfRow>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn GulfEconomiesFetcher,
) -> Result<PublishOutcome, EconomicSeederError> {
    let fetched = fetcher
        .fetch_basket(DEFAULT_COUNTRIES)
        .await
        .map_err(|e| EconomicSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EconomicSeederError::EmptyUpstream);
    }
    let mut rows: Vec<GulfCountryRow> = fetched
        .into_iter()
        .map(|r| GulfCountryRow {
            iso: r.iso,
            country: r.country,
            gdp_usd_billion: r.gdp_usd_billion,
            gdp_yoy_pct: r.gdp_yoy_pct,
            inflation_yoy_pct: r.inflation_yoy_pct,
            unemployment_pct: r.unemployment_pct,
            policy_rate_pct: r.policy_rate_pct,
            period: r.period,
        })
        .collect();
    rows.sort_by(|a, b| a.iso.cmp(&b.iso));

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = GulfSnapshot {
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
        rows: Vec<FetchedGulfRow>,
    }

    #[async_trait]
    impl GulfEconomiesFetcher for StaticFetcher {
        async fn fetch_basket(
            &self,
            _isos: &[&str],
        ) -> Result<Vec<FetchedGulfRow>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    fn row(iso: &str, country: &str) -> FetchedGulfRow {
        FetchedGulfRow {
            iso: iso.into(),
            country: country.into(),
            gdp_usd_billion: 1_000.0,
            gdp_yoy_pct: 2.5,
            inflation_yoy_pct: 1.8,
            unemployment_pct: 4.0,
            policy_rate_pct: 5.0,
            period: "2026-Q1".into(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "economic:gulf-economies:v1");
    }

    #[test]
    fn default_basket_has_seven_countries() {
        assert_eq!(DEFAULT_COUNTRIES.len(), 7);
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_rows() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                row("SAU", "Saudi Arabia"),
                row("IRN", "Iran"),
                row("ARE", "United Arab Emirates"),
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
        assert_eq!(isos, vec!["ARE", "IRN", "SAU"]);
    }
}
