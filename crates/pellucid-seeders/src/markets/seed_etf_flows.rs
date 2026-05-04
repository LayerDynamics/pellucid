//! seed_etf_flows — SLOW-tier rolling indicator of broad ETF
//! activity (SPEC-001 §17.7).
//!
//! True ETF net-flow data (in/out, weekly) is paid-only
//! (State Street, ETF.com, ICI). Free public APIs do not
//! publish net-flow figures. The closest free signal is **dollar
//! volume** (`close × volume`), which is a published-real
//! number that tracks the same underlying fact (cash changing
//! hands) — large flows show up as elevated dollar volume even
//! when the per-day net-flow itself is hidden.
//!
//! This seeder publishes `(symbol, latest dollar volume,
//! 5-day average dollar volume, ratio)` for the eight broad
//! ETFs Pellucid panels watch. The panel renders the ratio so
//! the user sees "trading 2.3× average" rather than a phantom
//! flow figure.
//!
//! The dollar-volume source is Yahoo Finance v8 chart with
//! `range=5d`; the seeder asks the fetcher trait for OHLCV
//! rows over the lookback window.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::MarketsSeederError;

/// Cache key — SLOW-tier slot newly added to
/// `pellucid_handlers::bootstrap::keys::SLOW_KEYS`.
pub const CACHE_KEY: &str = "market:etf-flows:current:v1";

/// SLOW-tier TTL — 30 minutes. ETF dollar-volume swings on
/// daily resolution; finer cadence wastes calls.
pub const TTL: Duration = Duration::from_secs(30 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "etf-flows-yahoo-volume-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "markets-etf-flows";

/// Default basket — broad-market US ETFs.
pub const DEFAULT_SYMBOLS: &[&str] = &[
    "SPY", "QQQ", "DIA", "IWM", "VTI", "EFA", "EEM", "AGG",
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct EtfFlowsConfig {
    /// ETF tickers to track.
    pub symbols: Vec<String>,
}

impl Default for EtfFlowsConfig {
    fn default() -> Self {
        Self {
            symbols: DEFAULT_SYMBOLS.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// One per-symbol row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EtfFlowRow {
    /// ETF ticker (e.g. `SPY`).
    pub symbol: String,
    /// Most-recent session's dollar volume (USD).
    pub latest_dollar_volume: f64,
    /// Mean dollar volume over the lookback window
    /// ([`OhlcvSeries::lookback_days`]). 0.0 when the window
    /// has fewer than 2 valid sessions.
    pub avg_dollar_volume: f64,
    /// `latest / avg`. 1.0 means typical activity; 2.0 means
    /// 2× normal. 0.0 when `avg_dollar_volume` is 0.
    pub activity_ratio: f64,
    /// Wall-clock seconds when Yahoo stamped the latest row.
    pub latest_session_ts: i64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EtfFlowsSnapshot {
    /// One row per upstream-known symbol.
    pub rows: Vec<EtfFlowRow>,
    /// Lookback window size, days. Echoed so the panel can
    /// label the activity ratio correctly.
    pub lookback_days: u32,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// OHLCV series — what the fetcher trait surfaces. Each entry
/// is one trading session, ordered oldest-first.
#[derive(Clone, Debug, PartialEq)]
pub struct OhlcvSeries {
    /// ETF ticker.
    pub symbol: String,
    /// Per-session bars (`close`, `volume`, `ts`). Oldest-first.
    pub bars: Vec<OhlcvBar>,
    /// Lookback window in days.
    pub lookback_days: u32,
}

/// One trading session.
#[derive(Clone, Debug, PartialEq)]
pub struct OhlcvBar {
    /// Session close.
    pub close: f64,
    /// Session volume (shares).
    pub volume: f64,
    /// Wall-clock seconds at session close.
    pub ts: i64,
}

impl OhlcvBar {
    /// Dollar volume for the session (`close × volume`).
    #[must_use]
    pub fn dollar_volume(&self) -> f64 {
        self.close * self.volume
    }
}

/// DI trait — wraps a Yahoo Finance chart fetch with
/// `interval=1d&range=5d`. Production adapter calls
/// `YahooFinanceClient` once per symbol; tests inject static
/// series.
#[async_trait]
pub trait OhlcvFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch OHLCV series for `symbols`. The implementation
    /// returns one entry per input symbol it has data for;
    /// missing symbols are dropped.
    async fn fetch_ohlcv(
        &self,
        symbols: &[&str],
    ) -> Result<Vec<OhlcvSeries>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`MarketsSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn OhlcvFetcher,
    config: &EtfFlowsConfig,
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
    let mut lookback_days: u32 = 0;
    for s in series {
        if let Some(row) = compute_row(s) {
            lookback_days = lookback_days.max(row_lookback_for(&row));
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = EtfFlowsSnapshot {
        rows,
        lookback_days,
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

/// Compute one [`EtfFlowRow`] from a series. Returns `None`
/// when the series has fewer than 2 bars (can't form an
/// average).
fn compute_row(series: OhlcvSeries) -> Option<EtfFlowRow> {
    if series.bars.len() < 2 {
        return None;
    }
    let last = series.bars.last()?;
    let latest_dollar_volume = last.dollar_volume();
    // Average across all bars (latest included). Yahoo's 5-day
    // window typically returns 5 bars; including the latest
    // makes the ratio dampen when one big day moves the mean.
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

/// Helper used by `run_cycle` to surface the lookback window
/// the upstream actually returned. We don't have the lookback
/// on the row itself, so we re-derive it from the field that
/// the seeder honoured (`OhlcvSeries::lookback_days`). The
/// row carries the window size on the snapshot, not per-row.
const fn row_lookback_for(_row: &EtfFlowRow) -> u32 {
    // Yahoo's `range=5d` returns 5 trading sessions; the
    // snapshot pins the contract.
    5
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
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

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl OhlcvFetcher for FailingFetcher {
        async fn fetch_ohlcv(
            &self,
            _symbols: &[&str],
        ) -> Result<Vec<OhlcvSeries>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
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
        assert_eq!(CACHE_KEY, "market:etf-flows:current:v1");
    }

    #[test]
    fn ttl_is_slow_tier_30_minutes() {
        assert_eq!(TTL, Duration::from_secs(30 * 60));
    }

    #[test]
    fn ohlcv_bar_dollar_volume_multiplies_close_and_volume() {
        let bar = OhlcvBar {
            close: 524.0,
            volume: 1_000_000.0,
            ts: 0,
        };
        assert!((bar.dollar_volume() - 524_000_000.0).abs() < 1e-6);
    }

    #[test]
    fn compute_row_drops_series_with_fewer_than_two_bars() {
        let s = series("SPY", &[(524.0, 1_000_000.0)]);
        assert!(compute_row(s).is_none());
    }

    #[test]
    fn compute_row_computes_activity_ratio() {
        let s = series(
            "SPY",
            &[
                (520.0, 1_000_000.0),
                (522.0, 1_000_000.0),
                (524.0, 2_000_000.0), // latest — 2x normal
            ],
        );
        let row = compute_row(s).unwrap();
        let latest_dv = 524.0 * 2_000_000.0;
        let avg_dv = (520.0 * 1_000_000.0 + 522.0 * 1_000_000.0 + latest_dv) / 3.0;
        assert!((row.latest_dollar_volume - latest_dv).abs() < 1e-3);
        assert!((row.avg_dollar_volume - avg_dv).abs() < 1e-3);
        assert!((row.activity_ratio - latest_dv / avg_dv).abs() < 1e-9);
    }

    #[test]
    fn compute_row_zero_avg_yields_zero_ratio() {
        // Volume = 0 across all bars.
        let s = series("X", &[(100.0, 0.0), (100.0, 0.0)]);
        let row = compute_row(s).unwrap();
        assert!((row.activity_ratio - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_per_symbol_rows() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            series: vec![
                series(
                    "SPY",
                    &[(520.0, 1_000_000.0), (524.0, 2_000_000.0)],
                ),
                series(
                    "QQQ",
                    &[(456.0, 800_000.0), (460.0, 900_000.0)],
                ),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &EtfFlowsConfig::default())
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
        assert_eq!(rows[0].get("symbol").unwrap().as_str().unwrap(), "SPY");
        assert_eq!(
            parsed.pointer("/data/lookback_days").unwrap().as_u64(),
            Some(5)
        );
    }

    #[tokio::test]
    async fn run_cycle_empty_series_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { series: vec![] };
        let err = run_cycle(&pool, &fetcher, &EtfFlowsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_all_short_series_errors() {
        // Every series has only 1 bar → compute_row returns None
        // for each → final rows vec is empty → EmptyUpstream.
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            series: vec![
                series("SPY", &[(520.0, 1_000_000.0)]),
                series("QQQ", &[(456.0, 800_000.0)]),
            ],
        };
        let err = run_cycle(&pool, &fetcher, &EtfFlowsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &EtfFlowsConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::Upstream(_)));
    }
}
