//! seed_gold_etf_flows — SLOW-tier rolling indicator of gold-
//! ETF activity (SPEC-001 §17.7).
//!
//! Same shape as [`crate::markets::seed_etf_flows`] but the
//! basket is gold-specific:
//!
//! | Symbol | Fund                                     | Sponsor       |
//! |--------|------------------------------------------|---------------|
//! | `GLD`  | SPDR Gold Shares                         | State Street  |
//! | `IAU`  | iShares Gold Trust                       | BlackRock     |
//! | `SGOL` | abrdn Physical Gold Shares ETF           | abrdn         |
//! | `GLDM` | SPDR Gold MiniShares                     | State Street  |
//! | `BAR`  | GraniteShares Gold Trust                 | GraniteShares |
//!
//! Same dollar-volume × lookback-window proxy as the broader
//! ETF-flow seeder; the panel renders "GLD trading 1.4×
//! average" vs "AGG trading 0.8× average" so users can see
//! gold-specific accumulation/distribution at a glance.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::seed_etf_flows::{
    EtfFlowRow, OhlcvBar, OhlcvFetcher, OhlcvSeries,
};
use crate::markets::MarketsSeederError;

/// Cache key — SLOW-tier slot newly added to
/// `pellucid_handlers::bootstrap::keys::SLOW_KEYS`.
pub const CACHE_KEY: &str = "market:gold-etf-flows:current:v1";

/// SLOW-tier TTL.
pub const TTL: Duration = Duration::from_secs(30 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "gold-etf-flows-yahoo-volume-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "markets-gold-etf-flows";

/// Default basket — five major gold-physical ETFs.
pub const DEFAULT_SYMBOLS: &[&str] = &["GLD", "IAU", "SGOL", "GLDM", "BAR"];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct GoldEtfFlowsConfig {
    /// Tickers to track.
    pub symbols: Vec<String>,
}

impl Default for GoldEtfFlowsConfig {
    fn default() -> Self {
        Self {
            symbols: DEFAULT_SYMBOLS.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// Published snapshot — wraps the same per-row shape as
/// [`crate::markets::seed_etf_flows`] so panels can reuse the
/// renderer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoldEtfFlowsSnapshot {
    /// One row per upstream-known symbol.
    pub rows: Vec<EtfFlowRow>,
    /// Lookback window in days (Yahoo `range=5d` → 5).
    pub lookback_days: u32,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Run one cycle.
///
/// # Errors
/// See [`MarketsSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn OhlcvFetcher,
    config: &GoldEtfFlowsConfig,
) -> Result<PublishOutcome, MarketsSeederError> {
    let symbol_refs: Vec<&str> = config.symbols.iter().map(String::as_str).collect();
    let series = fetcher
        .fetch_ohlcv(&symbol_refs)
        .await
        .map_err(|e| MarketsSeederError::Upstream(e.to_string()))?;
    if series.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let mut rows: Vec<EtfFlowRow> = Vec::with_capacity(series.len());
    for s in series {
        if let Some(row) = compute_row(s) {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = GoldEtfFlowsSnapshot {
        rows,
        lookback_days: 5,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(1_800_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "markets", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn compute_row(series: OhlcvSeries) -> Option<EtfFlowRow> {
    if series.bars.len() < 2 {
        return None;
    }
    let last = series.bars.last()?;
    let latest_dollar_volume = last.dollar_volume();
    let total_dv: f64 = series.bars.iter().map(OhlcvBar::dollar_volume).sum();
    let avg_dollar_volume = total_dv / series.bars.len() as f64;
    let activity_ratio = if avg_dollar_volume == 0.0 {
        0.0
    } else {
        latest_dollar_volume / avg_dollar_volume
    };
    Some(EtfFlowRow {
        symbol: series.symbol,
        latest_dollar_volume,
        avg_dollar_volume,
        activity_ratio,
        latest_session_ts: last.ts,
    })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        series: Vec<OhlcvSeries>,
    }

    #[async_trait]
    impl OhlcvFetcher for StaticFetcher {
        async fn fetch_ohlcv(
            &self,
            _symbols: &[&str],
        ) -> Result<Vec<OhlcvSeries>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.series.clone())
        }
    }

    fn series(symbol: &str, closes_volumes: &[(f64, f64)]) -> OhlcvSeries {
        OhlcvSeries {
            symbol: symbol.to_string(),
            bars: closes_volumes
                .iter()
                .enumerate()
                .map(|(i, (c, v))| OhlcvBar {
                    close: *c,
                    volume: *v,
                    ts: 1_714_060_800 + i as i64 * 86_400,
                })
                .collect(),
            lookback_days: 5,
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "market:gold-etf-flows:current:v1");
    }

    #[test]
    fn default_basket_includes_five_funds() {
        let cfg = GoldEtfFlowsConfig::default();
        assert_eq!(cfg.symbols.len(), 5);
        assert!(cfg.symbols.iter().any(|s| s == "GLD"));
        assert!(cfg.symbols.iter().any(|s| s == "IAU"));
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            series: vec![
                series(
                    "GLD",
                    &[(216.0, 5_000_000.0), (218.0, 7_000_000.0)],
                ),
                series(
                    "IAU",
                    &[(44.0, 8_000_000.0), (44.5, 8_500_000.0)],
                ),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &GoldEtfFlowsConfig::default())
            .await
            .unwrap();
        assert!(outcome.bytes_written > 0);

        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("symbol").unwrap().as_str().unwrap(), "GLD");
        let activity = rows[0].get("activity_ratio").unwrap().as_f64().unwrap();
        // Latest = 218*7M = 1.526B; avg = (216*5M + 218*7M) / 2 = 1.303B; ratio ≈ 1.171
        assert!((activity - 1.526e9 / 1.303e9).abs() < 1e-3);
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { series: vec![] };
        let err = run_cycle(&pool, &fetcher, &GoldEtfFlowsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_short_series_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            series: vec![series("GLD", &[(216.0, 5_000_000.0)])],
        };
        let err = run_cycle(&pool, &fetcher, &GoldEtfFlowsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }
}
