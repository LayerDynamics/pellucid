//! seed_national_debt — SLOW-tier U.S. national debt snapshot
//! (FRED `GFDEBTN` total + `GFDEGDQ188S` debt-to-GDP).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::economic::EconomicSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "economic:national-debt:v1";

/// SLOW-tier TTL — 24 h. FRED publishes both series quarterly.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "fred-national-debt-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "economic";

/// FRED series codes.
pub const SERIES_TOTAL: &str = "GFDEBTN";
/// FRED debt/GDP series.
pub const SERIES_DEBT_GDP: &str = "GFDEGDQ188S";

/// One observation pair.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DebtObservation {
    /// Quarter (`YYYY-QN`).
    pub period: String,
    /// Total debt in $B.
    pub total_billion_usd: f64,
    /// Debt-to-GDP percent.
    pub debt_to_gdp_pct: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NationalDebtSnapshot {
    /// Latest observation.
    pub latest: DebtObservation,
    /// Quarter-over-quarter delta of total debt ($B).
    pub qoq_delta_billion_usd: f64,
    /// Trailing observations sorted ascending by period.
    pub history: Vec<DebtObservation>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched series row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedDebtRow {
    /// Period.
    pub period: String,
    /// Total $B (raw FRED in $M is divided by 1000 by adapter).
    pub total_billion_usd: f64,
    /// Debt-to-GDP %.
    pub debt_to_gdp_pct: f64,
}

/// DI trait.
#[async_trait]
pub trait NationalDebtFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch trailing N quarters merging both series. Adapter
    /// matches the two FRED series by quarter and returns paired
    /// observations.
    async fn fetch_history(
        &self,
        history_quarters: usize,
    ) -> Result<Vec<FetchedDebtRow>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn NationalDebtFetcher,
    history_quarters: usize,
) -> Result<PublishOutcome, EconomicSeederError> {
    let fetched = fetcher
        .fetch_history(history_quarters)
        .await
        .map_err(|e| EconomicSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EconomicSeederError::EmptyUpstream);
    }
    let mut history: Vec<DebtObservation> = fetched
        .into_iter()
        .map(|r| DebtObservation {
            period: r.period,
            total_billion_usd: r.total_billion_usd,
            debt_to_gdp_pct: r.debt_to_gdp_pct,
        })
        .collect();
    history.sort_by(|a, b| a.period.cmp(&b.period));
    let Some(latest) = history.last().cloned() else {
        return Err(EconomicSeederError::EmptyUpstream);
    };
    let qoq = if history.len() >= 2 {
        latest.total_billion_usd - history[history.len() - 2].total_billion_usd
    } else {
        0.0
    };

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = NationalDebtSnapshot {
        latest,
        qoq_delta_billion_usd: qoq,
        history,
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
        rows: Vec<FetchedDebtRow>,
    }

    #[async_trait]
    impl NationalDebtFetcher for StaticFetcher {
        async fn fetch_history(
            &self,
            _n: usize,
        ) -> Result<Vec<FetchedDebtRow>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    fn obs(period: &str, total: f64, ratio: f64) -> FetchedDebtRow {
        FetchedDebtRow {
            period: period.into(),
            total_billion_usd: total,
            debt_to_gdp_pct: ratio,
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "economic:national-debt:v1");
    }

    #[tokio::test]
    async fn run_cycle_picks_latest_and_computes_qoq_delta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                obs("2025-Q4", 35_000.0, 121.0),
                obs("2026-Q1", 35_400.0, 122.5),
                obs("2025-Q3", 34_700.0, 120.5),
            ],
        };
        let _ = run_cycle(&pool, &fetcher, 8).await.unwrap();
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
            Some("2026-Q1"),
        );
        // QoQ = 35400 - 35000 = 400.
        assert!(
            (parsed
                .pointer("/data/qoq_delta_billion_usd")
                .and_then(serde_json::Value::as_f64)
                .unwrap()
                - 400.0)
                .abs()
                < 1e-6
        );
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, 8).await.unwrap_err();
        assert!(matches!(err, EconomicSeederError::EmptyUpstream));
    }
}
