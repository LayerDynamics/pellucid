//! seed_yield_curve — SLOW-tier U.S. Treasury yield curve.
//!
//! The webview's `YieldCurvePanel` (T4.2.9) renders the active
//! constant-maturity Treasury yield curve plus the headline
//! 2y/10y and 3m/10y inversion spreads. Per H3 the edge handler
//! reads this cache slot directly; this seeder is the only loop
//! piece that talks to FRED.
//!
//! ## FRED series
//!
//! | Code     | Maturity |
//! |----------|----------|
//! | DGS1MO   | 1m       |
//! | DGS3MO   | 3m       |
//! | DGS6MO   | 6m       |
//! | DGS1     | 1y       |
//! | DGS2     | 2y       |
//! | DGS5     | 5y       |
//! | DGS10    | 10y      |
//! | DGS30    | 30y      |

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::MarketsSeederError;

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "market:yield-curve:treasury:v1";

/// SLOW-tier TTL — 6 h. FRED publishes daily after market close;
/// 6 h is short enough that intraday boots get the prior day's
/// curve plus any same-day publish.
pub const TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "yield-curve-fred-cmt-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "markets";

/// FRED series codes the seeder fetches, in canonical maturity
/// order. The handler reads them in the same order so the
/// panel's curve chart renders left-to-right correctly.
pub const SERIES_CODES: &[&str] = &[
    "DGS1MO", "DGS3MO", "DGS6MO", "DGS1", "DGS2", "DGS5", "DGS10", "DGS30",
];

/// Human-readable maturity labels matching [`SERIES_CODES`].
pub const MATURITY_LABELS: &[&str] =
    &["1M", "3M", "6M", "1Y", "2Y", "5Y", "10Y", "30Y"];

/// Maturity expressed in months — for spread computation and
/// chart x-axis positioning.
pub const MATURITY_MONTHS: &[u32] = &[1, 3, 6, 12, 24, 60, 120, 360];

/// Run-time configuration. The default basket is the canonical
/// constant-maturity set above; tests pass a shorter slice so
/// fixtures stay small.
#[derive(Clone, Debug)]
pub struct YieldCurveConfig {
    /// Series codes to fetch (FRED ids).
    pub series_codes: Vec<String>,
    /// Maturity labels in the same order.
    pub maturity_labels: Vec<String>,
    /// Maturity in months in the same order.
    pub maturity_months: Vec<u32>,
}

impl Default for YieldCurveConfig {
    fn default() -> Self {
        Self {
            series_codes: SERIES_CODES.iter().map(|s| (*s).to_string()).collect(),
            maturity_labels: MATURITY_LABELS.iter().map(|s| (*s).to_string()).collect(),
            maturity_months: MATURITY_MONTHS.to_vec(),
        }
    }
}

/// One yield-curve point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct YieldPoint {
    /// FRED series code.
    pub series_code: String,
    /// Maturity label.
    pub maturity_label: String,
    /// Maturity in months.
    pub maturity_months: u32,
    /// Yield in percent. 0.0 when FRED reported `.` (no value).
    pub yield_pct: f64,
    /// Observation date (`YYYY-MM-DD`).
    pub observation_date: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct YieldCurveSnapshot {
    /// Points in canonical maturity order. Missing points are
    /// silently skipped (FRED occasionally omits the very-short
    /// or very-long end on a particular day).
    pub points: Vec<YieldPoint>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled yield observation — mirrors `pellucid_streams::FredObservation`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedYieldObservation {
    /// FRED series code.
    pub series_code: String,
    /// Yield in percent.
    pub yield_pct: f64,
    /// Observation date.
    pub observation_date: String,
}

/// DI trait — wraps `pellucid_streams::FredClient::fetch_observations`.
#[async_trait]
pub trait YieldCurveFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest observation for each series code.
    /// Implementations return `Ok(None)` for series that the
    /// upstream had no value for; the seeder drops them.
    async fn fetch_latest(
        &self,
        series_codes: &[&str],
    ) -> Result<Vec<Option<FetchedYieldObservation>>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// - [`MarketsSeederError::Upstream`] when the fetcher errors.
/// - [`MarketsSeederError::EmptyUpstream`] when every series
///   returned `None`.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn YieldCurveFetcher,
    config: &YieldCurveConfig,
) -> Result<PublishOutcome, MarketsSeederError> {
    let codes: Vec<&str> = config.series_codes.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_latest(&codes)
        .await
        .map_err(|e| MarketsSeederError::Upstream(e.to_string()))?;

    let mut points: Vec<YieldPoint> = Vec::with_capacity(fetched.len());
    for (i, obs) in fetched.into_iter().enumerate() {
        let Some(o) = obs else { continue };
        let Some(label) = config.maturity_labels.get(i).cloned() else {
            continue;
        };
        let months = config.maturity_months.get(i).copied().unwrap_or(0);
        points.push(YieldPoint {
            series_code: o.series_code,
            maturity_label: label,
            maturity_months: months,
            yield_pct: o.yield_pct,
            observation_date: o.observation_date,
        });
    }
    if points.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = YieldCurveSnapshot {
        points,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(21_600_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.points.len()).unwrap_or(0),
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
        rows: Vec<Option<FetchedYieldObservation>>,
    }

    #[async_trait]
    impl YieldCurveFetcher for StaticFetcher {
        async fn fetch_latest(
            &self,
            _series_codes: &[&str],
        ) -> Result<Vec<Option<FetchedYieldObservation>>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl YieldCurveFetcher for FailingFetcher {
        async fn fetch_latest(
            &self,
            _series_codes: &[&str],
        ) -> Result<Vec<Option<FetchedYieldObservation>>, Box<dyn std::error::Error + Send + Sync>>
        {
            Err("upstream down".into())
        }
    }

    fn obs(series: &str, yld: f64, date: &str) -> FetchedYieldObservation {
        FetchedYieldObservation {
            series_code: series.into(),
            yield_pct: yld,
            observation_date: date.into(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "market:yield-curve:treasury:v1");
    }

    #[test]
    fn ttl_is_six_hours() {
        assert_eq!(TTL, Duration::from_secs(21_600));
    }

    #[test]
    fn default_basket_is_eight_canonical_maturities() {
        let cfg = YieldCurveConfig::default();
        assert_eq!(cfg.series_codes.len(), 8);
        assert_eq!(cfg.maturity_labels.len(), 8);
        assert_eq!(cfg.maturity_months.len(), 8);
        assert_eq!(cfg.series_codes[0], "DGS1MO");
        assert_eq!(cfg.series_codes[7], "DGS30");
    }

    #[tokio::test]
    async fn run_cycle_writes_points_in_maturity_order_and_skips_missing() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                Some(obs("DGS1MO", 5.30, "2026-05-05")),
                None, // DGS3MO missing
                Some(obs("DGS6MO", 5.10, "2026-05-05")),
                Some(obs("DGS1", 4.95, "2026-05-05")),
                Some(obs("DGS2", 4.70, "2026-05-05")),
                Some(obs("DGS5", 4.30, "2026-05-05")),
                Some(obs("DGS10", 4.20, "2026-05-05")),
                Some(obs("DGS30", 4.40, "2026-05-05")),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &YieldCurveConfig::default())
            .await
            .unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) = sqlx::query_as(
            "SELECT payload FROM kv_envelope WHERE cache_key = ?",
        )
        .bind(CACHE_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let pts = parsed.pointer("/data/points").unwrap().as_array().unwrap();
        // 7 points (DGS3MO skipped).
        assert_eq!(pts.len(), 7);
        let codes: Vec<&str> = pts
            .iter()
            .map(|p| p.get("series_code").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(codes[0], "DGS1MO");
        assert_eq!(codes[1], "DGS6MO");
    }

    #[tokio::test]
    async fn run_cycle_all_missing_returns_empty_upstream_error() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![None; 8],
        };
        let err = run_cycle(&pool, &fetcher, &YieldCurveConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &YieldCurveConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta_with_cascade_group() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![Some(obs("DGS1MO", 5.30, "2026-05-05")); 8],
        };
        let _ = run_cycle(&pool, &fetcher, &YieldCurveConfig::default())
            .await
            .unwrap();
        let row: (String, String) = sqlx::query_as(
            "SELECT source_version, cascade_group FROM seed_meta WHERE cache_key = ?",
        )
        .bind(CACHE_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, SOURCE_VERSION);
        assert_eq!(row.1, CASCADE_GROUP);
    }
}
