//! seed_liquidity_shifts — SLOW-tier weekly snapshot of the
//! three FRED series the `LiquidityShiftsPanel` (T4.2.11)
//! reads to render USD liquidity dynamics.
//!
//! ## FRED series
//!
//! | Code      | Description                                      |
//! |-----------|--------------------------------------------------|
//! | WALCL     | Total Federal Reserve assets ($B, weekly)        |
//! | M2SL      | M2 money stock, seasonally adjusted ($B, weekly) |
//! | RRPONTSYD | Overnight reverse repo facility ($B, daily)      |
//!
//! `liquidity = WALCL - RRPONTSYD` proxies "net Fed liquidity"
//! the panel charts; M2 is the broader money stock for context.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::MarketsSeederError;

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "market:liquidity-shifts:v1";

/// SLOW-tier TTL — 12 h. WALCL publishes Thursdays; M2 monthly;
/// RRPONTSYD daily — 12 h captures the daily refresh while
/// keeping the read budget low.
pub const TTL: Duration = Duration::from_secs(12 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "liquidity-shifts-fred-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "markets";

/// FRED series codes the seeder fetches.
pub const SERIES_CODES: &[&str] = &["WALCL", "M2SL", "RRPONTSYD"];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct LiquidityShiftsConfig {
    /// FRED series codes to fetch.
    pub series_codes: Vec<String>,
}

impl Default for LiquidityShiftsConfig {
    fn default() -> Self {
        Self {
            series_codes: SERIES_CODES.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// One series' latest observation + week-over-week delta.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiquiditySeriesRow {
    /// FRED series code.
    pub series_code: String,
    /// Latest observation value (in series units; FRED publishes
    /// these in $B).
    pub latest_value: f64,
    /// Date of the latest observation (`YYYY-MM-DD`).
    pub latest_date: String,
    /// Prior-period value (the observation before `latest`).
    /// `None` when the upstream returned only one observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_value: Option<f64>,
    /// Latest minus prior. `None` when `prior_value` is None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_delta: Option<f64>,
    /// `period_delta / prior * 100`. `None` when prior is missing
    /// or zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_delta_pct: Option<f64>,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiquidityShiftsSnapshot {
    /// Series rows in canonical order: WALCL, M2SL, RRPONTSYD.
    pub series: Vec<LiquiditySeriesRow>,
    /// `WALCL - RRPONTSYD` net-liquidity proxy in $B. `0.0`
    /// when either series is missing.
    pub net_liquidity_billion_usd: f64,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled FRED observation pair (latest + prior).
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedLiquiditySeries {
    /// FRED series code.
    pub series_code: String,
    /// Latest observation value.
    pub latest_value: f64,
    /// Latest observation date.
    pub latest_date: String,
    /// Prior-period observation value, when present.
    pub prior_value: Option<f64>,
}

/// DI trait — wraps `pellucid_streams::FredClient::fetch_observations`
/// with a "give me the most recent two observations" contract.
#[async_trait]
pub trait LiquidityShiftsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest + prior observation per series. Each
    /// `Option` is `None` for a series the upstream had no value
    /// for.
    async fn fetch_latest_pair(
        &self,
        series_codes: &[&str],
    ) -> Result<Vec<Option<FetchedLiquiditySeries>>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Compute period delta + percent. Pure, exported for tests.
#[must_use]
pub fn period_delta_components(latest: f64, prior: Option<f64>) -> (Option<f64>, Option<f64>) {
    let Some(p) = prior else {
        return (None, None);
    };
    let delta = latest - p;
    let pct = if p.abs() < f64::EPSILON {
        None
    } else {
        Some(delta / p * 100.0)
    };
    (Some(delta), pct)
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn LiquidityShiftsFetcher,
    config: &LiquidityShiftsConfig,
) -> Result<PublishOutcome, MarketsSeederError> {
    let codes: Vec<&str> = config.series_codes.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_latest_pair(&codes)
        .await
        .map_err(|e| MarketsSeederError::Upstream(e.to_string()))?;

    let mut walcl: Option<f64> = None;
    let mut rrp: Option<f64> = None;
    let mut series: Vec<LiquiditySeriesRow> = Vec::with_capacity(fetched.len());
    for (i, opt) in fetched.into_iter().enumerate() {
        let Some(o) = opt else { continue };
        let code = config
            .series_codes
            .get(i)
            .cloned()
            .unwrap_or(o.series_code.clone());
        let (delta, pct) = period_delta_components(o.latest_value, o.prior_value);
        if code.eq_ignore_ascii_case("WALCL") {
            walcl = Some(o.latest_value);
        }
        if code.eq_ignore_ascii_case("RRPONTSYD") {
            rrp = Some(o.latest_value);
        }
        series.push(LiquiditySeriesRow {
            series_code: code,
            latest_value: o.latest_value,
            latest_date: o.latest_date,
            prior_value: o.prior_value,
            period_delta: delta,
            period_delta_pct: pct,
        });
    }
    if series.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }
    let net_liquidity = match (walcl, rrp) {
        (Some(w), Some(r)) => w - r,
        _ => 0.0,
    };

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = LiquidityShiftsSnapshot {
        series,
        net_liquidity_billion_usd: net_liquidity,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(43_200_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.series.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "markets", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<Option<FetchedLiquiditySeries>>,
    }

    #[async_trait]
    impl LiquidityShiftsFetcher for StaticFetcher {
        async fn fetch_latest_pair(
            &self,
            _series_codes: &[&str],
        ) -> Result<Vec<Option<FetchedLiquiditySeries>>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    fn fls(code: &str, latest: f64, prior: Option<f64>) -> FetchedLiquiditySeries {
        FetchedLiquiditySeries {
            series_code: code.into(),
            latest_value: latest,
            latest_date: "2026-05-01".into(),
            prior_value: prior,
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "market:liquidity-shifts:v1");
    }

    #[test]
    fn default_basket_is_three_series() {
        assert_eq!(LiquidityShiftsConfig::default().series_codes.len(), 3);
    }

    #[test]
    fn period_delta_components_handles_missing_prior() {
        let (d, p) = period_delta_components(100.0, None);
        assert!(d.is_none());
        assert!(p.is_none());
    }

    #[test]
    fn period_delta_components_zero_prior_yields_no_pct() {
        let (d, p) = period_delta_components(50.0, Some(0.0));
        assert_eq!(d, Some(50.0));
        assert!(p.is_none());
    }

    #[test]
    fn period_delta_components_normal_case() {
        let (d, p) = period_delta_components(110.0, Some(100.0));
        assert_eq!(d, Some(10.0));
        assert!((p.unwrap() - 10.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_net_liquidity_proxy() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                Some(fls("WALCL", 7_200.0, Some(7_180.0))),
                Some(fls("M2SL", 21_000.0, Some(20_980.0))),
                Some(fls("RRPONTSYD", 450.0, Some(480.0))),
            ],
        };
        let _ = run_cycle(&pool, &fetcher, &LiquidityShiftsConfig::default())
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        // Net liquidity = WALCL - RRPONTSYD = 7200 - 450 = 6750.
        let net = parsed
            .pointer("/data/net_liquidity_billion_usd")
            .unwrap()
            .as_f64()
            .unwrap();
        assert!((net - 6_750.0).abs() < 1e-9);
        let series = parsed.pointer("/data/series").unwrap().as_array().unwrap();
        assert_eq!(series.len(), 3);
    }

    #[tokio::test]
    async fn run_cycle_all_missing_returns_empty_upstream_error() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![None, None, None],
        };
        let err = run_cycle(&pool, &fetcher, &LiquidityShiftsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }
}
