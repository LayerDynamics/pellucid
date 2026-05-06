//! seed_semiconductor_ppi — SLOW-tier snapshot of US Bureau
//! of Labor Statistics Producer Price Indices for the
//! semiconductor + storage industry, fetched from FRED.
//!
//! ## Why this seeder exists alongside `seed_memory_market`
//!
//! `seed_memory_market` publishes an equity-side proxy (MU /
//! WDC / STX / Samsung stock prices) for the paywalled DRAM
//! spot market. This seeder publishes the supply-side
//! complement: real US producer prices for the underlying
//! industry, sourced from St. Louis Fed FRED. PPI moves
//! monthly and lags consumer-facing spot prices by 1-3
//! months, but it is the canonical real numeric series that
//! tracks the full industry — equity prices alone over- or
//! under-shoot industry pricing because they bake in macro
//! and AI-spend expectations.
//!
//! The two seeders together give the panel two free signals
//! for the same underlying market.
//!
//! ## FRED series
//!
//! - `PCU3344133441`     — Semiconductor & Other Electronic
//!   Component Manufacturing PPI (Dec 1984-present).
//! - `PCU334413334413`   — Semiconductor & Related Device
//!   Manufacturing PPI (Jan 1967-present).
//! - `PCU334112334112`   — Computer Storage Device
//!   Manufacturing PPI.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::technology::TechnologySeederError;

/// Cache key — NEW SLOW tier slot added by T3.8 expansion.
pub const CACHE_KEY: &str = "technology:semiconductor-ppi:monthly:v1";

/// SLOW-tier TTL — 12 hours. PPI updates monthly; 12-hour
/// cache lets the seeder catch the BLS release on the
/// release day without burning calls in between.
pub const TTL: Duration = Duration::from_secs(12 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "semiconductor-ppi-fred-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "technology-ppi";

/// Default series basket — the three industry-level PPIs that
/// move together but capture different slices.
pub const DEFAULT_SERIES_IDS: &[&str] = &[
    "PCU3344133441",   // Semiconductor & Other Electronic Component Manufacturing
    "PCU334413334413", // Semiconductor & Related Device Manufacturing
    "PCU334112334112", // Computer Storage Device Manufacturing
];

/// Default lookback — 24 months. Enough for a year-over-year
/// sparkline + a 12-month YoY delta calc.
pub const DEFAULT_LOOKBACK_MONTHS: u32 = 24;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct SemiconductorPpiConfig {
    /// FRED series ids to fetch.
    pub series_ids: Vec<String>,
    /// Lookback window in months.
    pub lookback_months: u32,
}

impl Default for SemiconductorPpiConfig {
    fn default() -> Self {
        Self {
            series_ids: DEFAULT_SERIES_IDS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            lookback_months: DEFAULT_LOOKBACK_MONTHS,
        }
    }
}

/// One PPI reading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PpiReading {
    /// `YYYY-MM-01` observation date.
    pub date: String,
    /// PPI value (`None` when FRED reports a `"."` blank).
    pub value: Option<f64>,
}

/// One per-series row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesRow {
    /// FRED series id.
    pub series_id: String,
    /// Server-side total observation count for the series.
    pub total_count: u64,
    /// Most-recent N observations (newest-first).
    pub readings: Vec<PpiReading>,
    /// Headline figure: most-recent observation with a numeric
    /// value. `None` only if the series has no numeric values
    /// in the lookback window.
    pub latest: Option<PpiReading>,
    /// Year-over-year percent change vs the reading 12 months
    /// before `latest`. `None` when the lookback window is
    /// shorter than 13 months or when either anchor is `"."`.
    pub yoy_percent_change: Option<f64>,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemiconductorPpiSnapshot {
    /// Per-series rows in input order.
    pub rows: Vec<SeriesRow>,
    /// Documented note about the data source.
    pub source_note: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled FRED observation — mirrors `pellucid_streams::FredObservation`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedObservation {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// Numeric value (`None` for `"."` placeholders).
    pub value: Option<f64>,
}

/// Distilled FRED series — mirrors `pellucid_streams::FredSeries`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedSeries {
    /// Series id.
    pub series_id: String,
    /// Total count.
    pub count: u64,
    /// Observations newest-first.
    pub observations: Vec<FetchedObservation>,
}

/// DI trait — wraps `pellucid_streams::FredClient::fetch_observations`.
#[async_trait]
pub trait FredFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the most-recent `limit` observations for `series_id`.
    async fn fetch_observations(
        &self,
        series_id: &str,
        limit: u32,
    ) -> Result<FetchedSeries, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`TechnologySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn FredFetcher,
    config: &SemiconductorPpiConfig,
) -> Result<PublishOutcome, TechnologySeederError> {
    let mut rows: Vec<SeriesRow> = Vec::with_capacity(config.series_ids.len());
    for series_id in &config.series_ids {
        let fetched = fetcher
            .fetch_observations(series_id, config.lookback_months)
            .await
            .map_err(|e| TechnologySeederError::Upstream(e.to_string()))?;
        if let Some(row) = build_series_row(fetched) {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return Err(TechnologySeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = SemiconductorPpiSnapshot {
        rows,
        source_note: "Industry-level Producer Price Indices from US BLS \
            via FRED (St. Louis Fed). PPI lags consumer-facing DRAM/NAND \
            spot prices by 1-3 months but tracks the canonical real \
            numeric series for the manufacturing industry. Pellucid pairs \
            this with the equity-side proxy in seed_memory_market for a \
            two-signal view of the memory market."
            .into(),
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
    let outcome = atomic_publish(pool, "technology", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn build_series_row(fetched: FetchedSeries) -> Option<SeriesRow> {
    let readings: Vec<PpiReading> = fetched
        .observations
        .iter()
        .map(|o| PpiReading {
            date: o.date.clone(),
            value: o.value,
        })
        .collect();
    let latest = readings.iter().find(|r| r.value.is_some()).cloned();
    let yoy_percent_change = compute_yoy(&readings);
    Some(SeriesRow {
        series_id: fetched.series_id,
        total_count: fetched.count,
        readings,
        latest,
        yoy_percent_change,
    })
}

/// Year-over-year percent change. Returns `None` when the
/// lookback has fewer than 13 readings, when either anchor
/// is `None`, or when the older anchor is 0.
fn compute_yoy(readings: &[PpiReading]) -> Option<f64> {
    if readings.len() < 13 {
        return None;
    }
    let latest = readings[0].value?;
    let year_ago = readings[12].value?;
    if year_ago == 0.0 {
        return None;
    }
    Some((latest - year_ago) / year_ago * 100.0)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        responses: std::collections::HashMap<String, FetchedSeries>,
    }

    #[async_trait]
    impl FredFetcher for StaticFetcher {
        async fn fetch_observations(
            &self,
            series_id: &str,
            _limit: u32,
        ) -> Result<FetchedSeries, Box<dyn std::error::Error + Send + Sync>> {
            self.responses
                .get(series_id)
                .cloned()
                .ok_or_else(|| format!("no fixture for {series_id}").into())
        }
    }

    fn series(id: &str, observations: &[(&str, Option<f64>)]) -> FetchedSeries {
        FetchedSeries {
            series_id: id.into(),
            count: observations.len() as u64,
            observations: observations
                .iter()
                .map(|(d, v)| FetchedObservation {
                    date: (*d).to_string(),
                    value: *v,
                })
                .collect(),
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "technology:semiconductor-ppi:monthly:v1");
    }

    #[test]
    fn ttl_is_twelve_hours() {
        assert_eq!(TTL, Duration::from_secs(12 * 60 * 60));
    }

    #[test]
    fn default_basket_includes_three_series() {
        let cfg = SemiconductorPpiConfig::default();
        assert_eq!(cfg.series_ids.len(), 3);
        assert!(cfg.series_ids.iter().any(|s| s == "PCU3344133441"));
        assert!(cfg.series_ids.iter().any(|s| s == "PCU334413334413"));
        assert!(cfg.series_ids.iter().any(|s| s == "PCU334112334112"));
    }

    #[test]
    fn compute_yoy_with_full_window() {
        let readings: Vec<PpiReading> = (0..13)
            .map(|i| PpiReading {
                date: format!("2026-{:02}-01", i + 1),
                value: Some(100.0 + i as f64),
            })
            .collect();
        // latest = 100 + 0 = 100 (newest is index 0); year ago = 100 + 12 = 112.
        // yoy = (100 - 112) / 112 * 100 ≈ -10.71%
        let yoy = compute_yoy(&readings).unwrap();
        assert!((yoy - ((100.0 - 112.0) / 112.0 * 100.0)).abs() < 1e-9);
    }

    #[test]
    fn compute_yoy_short_window_returns_none() {
        let readings: Vec<PpiReading> = (0..12)
            .map(|i| PpiReading {
                date: format!("2026-{:02}-01", i + 1),
                value: Some(100.0),
            })
            .collect();
        assert!(compute_yoy(&readings).is_none());
    }

    #[test]
    fn compute_yoy_skips_dot_anchors() {
        let mut readings: Vec<PpiReading> = (0..13)
            .map(|i| PpiReading {
                date: format!("2026-{:02}-01", i + 1),
                value: Some(100.0 + i as f64),
            })
            .collect();
        readings[12].value = None;
        assert!(compute_yoy(&readings).is_none());
    }

    #[tokio::test]
    async fn run_cycle_writes_one_row_per_series() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            "PCU3344133441".into(),
            series(
                "PCU3344133441",
                &[("2026-04-01", Some(117.5)), ("2026-03-01", Some(115.0))],
            ),
        );
        responses.insert(
            "PCU334413334413".into(),
            series(
                "PCU334413334413",
                &[("2026-04-01", Some(99.2)), ("2026-03-01", Some(98.0))],
            ),
        );
        responses.insert(
            "PCU334112334112".into(),
            series(
                "PCU334112334112",
                &[("2026-04-01", Some(105.0)), ("2026-03-01", Some(104.0))],
            ),
        );
        let fetcher = StaticFetcher { responses };
        let _ = run_cycle(&pool, &fetcher, &SemiconductorPpiConfig::default())
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 3);
        let pcu = rows
            .iter()
            .find(|r| r.get("series_id").unwrap().as_str() == Some("PCU3344133441"))
            .unwrap();
        let latest_value = pcu.pointer("/latest/value").unwrap().as_f64().unwrap();
        assert!((latest_value - 117.5).abs() < 1e-9);
        let note = parsed
            .pointer("/data/source_note")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(note.contains("FRED"));
        assert!(note.contains("BLS"));
    }

    #[tokio::test]
    async fn run_cycle_empty_responses_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            responses: std::collections::HashMap::new(),
        };
        let cfg = SemiconductorPpiConfig {
            series_ids: vec![],
            lookback_months: 24,
        };
        let err = run_cycle(&pool, &fetcher, &cfg).await.unwrap_err();
        assert!(matches!(err, TechnologySeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        // One series in the basket, but no fixture → the fetcher
        // returns an error for that id.
        let fetcher = StaticFetcher {
            responses: std::collections::HashMap::new(),
        };
        let cfg = SemiconductorPpiConfig {
            series_ids: vec!["PCU3344133441".into()],
            lookback_months: 24,
        };
        let err = run_cycle(&pool, &fetcher, &cfg).await.unwrap_err();
        assert!(matches!(err, TechnologySeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            "PCU3344133441".into(),
            series("PCU3344133441", &[("2026-04-01", Some(117.5))]),
        );
        let fetcher = StaticFetcher { responses };
        let cfg = SemiconductorPpiConfig {
            series_ids: vec!["PCU3344133441".into()],
            lookback_months: 24,
        };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
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
