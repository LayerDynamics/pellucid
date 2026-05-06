//! seed_market_history — SLOW-tier snapshot of historical OHLC
//! bars for the broad-market basket.
//!
//! The webview's `StockBacktestPanel` (T4.2.3) needs ~3 months
//! of daily bars per symbol to compute a real moving-average
//! cross backtest. Per SPEC-001 §24's H3 fix, edge handlers MUST
//! NEVER call upstreams synchronously — this seeder publishes
//! the history snapshot every 24 h so the backtest handler is
//! a pure cache reader.
//!
//! Symbols match `seed_market_quotes::DEFAULT_SYMBOLS` so the
//! analytics + backtest views agree on the basket.
//!
//! ## Cache shape
//!
//! ```jsonc
//! {
//!   "symbols": [{
//!     "symbol": "SPY",
//!     "bars": [
//!       { "time_secs": 1714060800, "open": 522.0, "high": 525.0,
//!         "low": 521.0, "close": 524.0, "volume": 12345678 },
//!       …
//!     ]
//!   }],
//!   "interval": "1d",
//!   "range": "3mo",
//!   "assembled_at_ms": 1746360000000
//! }
//! ```

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::seed_market_quotes::DEFAULT_SYMBOLS;
use crate::markets::MarketsSeederError;

/// Cache key — written to SLOW tier.
pub const CACHE_KEY: &str = "market:history:default-basket:v1";

/// SLOW-tier TTL — 24 h.
pub const TTL: Duration = Duration::from_secs(60 * 60 * 24);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "market-history-yahoo-1d-3mo-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "markets";

/// Default Yahoo `interval` parameter.
pub const DEFAULT_INTERVAL: &str = "1d";

/// Default Yahoo `range` parameter — 3 months gives ~63 trading
/// days, enough for a reliable SMA-30 baseline.
pub const DEFAULT_RANGE: &str = "3mo";

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct MarketHistoryConfig {
    /// Symbols to fetch. Defaults to the same basket as
    /// `seed_market_quotes`.
    pub symbols: Vec<String>,
    /// Yahoo `interval` parameter.
    pub interval: String,
    /// Yahoo `range` parameter.
    pub range: String,
}

impl Default for MarketHistoryConfig {
    fn default() -> Self {
        Self {
            symbols: DEFAULT_SYMBOLS.iter().map(|s| (*s).to_string()).collect(),
            interval: DEFAULT_INTERVAL.to_string(),
            range: DEFAULT_RANGE.to_string(),
        }
    }
}

/// One historical bar in the published snapshot. Mirrors
/// `pellucid_streams::YahooBar`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryBarRow {
    /// Wall-clock seconds at the bar's open.
    pub time_secs: i64,
    /// Open price.
    pub open: f64,
    /// Highest traded price during the bar.
    pub high: f64,
    /// Lowest traded price during the bar.
    pub low: f64,
    /// Close price.
    pub close: f64,
    /// Volume — `None` when the upstream omitted it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<i64>,
}

/// Per-symbol bar series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SymbolHistory {
    /// Ticker symbol.
    pub symbol: String,
    /// Bars sorted ascending by `time_secs`.
    pub bars: Vec<HistoryBarRow>,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketHistorySnapshot {
    /// One series per fetched symbol. Symbols whose upstream
    /// returned no bars are omitted entirely.
    pub symbols: Vec<SymbolHistory>,
    /// Echo of the `interval` request parameter.
    pub interval: String,
    /// Echo of the `range` request parameter.
    pub range: String,
    /// Wall-clock ms when the seeder assembled.
    pub assembled_at_ms: i64,
}

/// Distilled bar — mirrors `pellucid_streams::YahooBar` without
/// pulling streams as a hard dep on this crate.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedBar {
    /// Wall-clock seconds.
    pub time_secs: i64,
    /// Open.
    pub open: f64,
    /// High.
    pub high: f64,
    /// Low.
    pub low: f64,
    /// Close.
    pub close: f64,
    /// Volume.
    pub volume: Option<i64>,
}

/// DI trait — wraps `pellucid_streams::YahooFinanceClient::fetch_history`.
#[async_trait]
pub trait MarketHistoryFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the bar series for `symbol`. `Ok(Vec::new())` is
    /// reserved for "no upstream data" and is treated as a
    /// per-symbol skip; `Err` is reserved for transient
    /// upstream failures.
    async fn fetch_history(
        &self,
        symbol: &str,
        interval: &str,
        range: &str,
    ) -> Result<Vec<FetchedBar>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// Returns [`MarketsSeederError::Upstream`] when ALL symbols
/// fail; returns [`MarketsSeederError::EmptyUpstream`] when the
/// fan-out completes but no symbol returned a bar series.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn MarketHistoryFetcher,
    config: &MarketHistoryConfig,
) -> Result<PublishOutcome, MarketsSeederError> {
    let mut series: Vec<SymbolHistory> = Vec::with_capacity(config.symbols.len());
    let mut last_err: Option<String> = None;
    let mut any_ok = false;
    for symbol in &config.symbols {
        match fetcher
            .fetch_history(symbol, &config.interval, &config.range)
            .await
        {
            Ok(bars) if !bars.is_empty() => {
                any_ok = true;
                series.push(SymbolHistory {
                    symbol: symbol.clone(),
                    bars: bars
                        .into_iter()
                        .map(|b| HistoryBarRow {
                            time_secs: b.time_secs,
                            open: b.open,
                            high: b.high,
                            low: b.low,
                            close: b.close,
                            volume: b.volume,
                        })
                        .collect(),
                });
            }
            Ok(_) => {
                any_ok = true;
            }
            Err(e) => {
                last_err = Some(e.to_string());
            }
        }
    }

    if !any_ok {
        if let Some(e) = last_err {
            return Err(MarketsSeederError::Upstream(e));
        }
        return Err(MarketsSeederError::EmptyUpstream);
    }
    if series.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = MarketHistorySnapshot {
        symbols: series,
        interval: config.interval.clone(),
        range: config.range.clone(),
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(86_400_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.symbols.len()).unwrap_or(0),
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
    use std::sync::Mutex;

    #[derive(Debug)]
    struct StaticFetcher {
        rows_by_symbol: Vec<(String, Vec<FetchedBar>)>,
        last_calls: Mutex<Vec<(String, String, String)>>,
    }

    #[async_trait]
    impl MarketHistoryFetcher for StaticFetcher {
        async fn fetch_history(
            &self,
            symbol: &str,
            interval: &str,
            range: &str,
        ) -> Result<Vec<FetchedBar>, Box<dyn std::error::Error + Send + Sync>> {
            self.last_calls.lock().unwrap().push((
                symbol.to_string(),
                interval.to_string(),
                range.to_string(),
            ));
            for (s, bars) in &self.rows_by_symbol {
                if s == symbol {
                    return Ok(bars.clone());
                }
            }
            Ok(Vec::new())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl MarketHistoryFetcher for FailingFetcher {
        async fn fetch_history(
            &self,
            _symbol: &str,
            _interval: &str,
            _range: &str,
        ) -> Result<Vec<FetchedBar>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn bar(t: i64, o: f64, h: f64, l: f64, c: f64) -> FetchedBar {
        FetchedBar {
            time_secs: t,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: Some(1_000),
        }
    }

    #[test]
    fn cache_key_matches_slow_keys_slot() {
        assert_eq!(CACHE_KEY, "market:history:default-basket:v1");
    }

    #[test]
    fn ttl_is_24h() {
        assert_eq!(TTL, Duration::from_secs(86_400));
    }

    #[test]
    fn default_interval_and_range() {
        let cfg = MarketHistoryConfig::default();
        assert_eq!(cfg.interval, "1d");
        assert_eq!(cfg.range, "3mo");
        assert_eq!(cfg.symbols.len(), 8);
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_one_series_per_symbol() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows_by_symbol: vec![
                ("SPY".into(), vec![bar(1, 100.0, 101.0, 99.0, 100.5)]),
                ("QQQ".into(), vec![bar(1, 200.0, 201.0, 199.0, 200.5)]),
            ],
            last_calls: Mutex::new(Vec::new()),
        };
        let cfg = MarketHistoryConfig {
            symbols: vec!["SPY".into(), "QQQ".into()],
            interval: "1d".into(),
            range: "3mo".into(),
        };
        let outcome = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let series = parsed.pointer("/data/symbols").unwrap().as_array().unwrap();
        assert_eq!(series.len(), 2);
    }

    #[tokio::test]
    async fn run_cycle_skips_symbols_with_no_bars() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows_by_symbol: vec![
                ("SPY".into(), vec![bar(1, 100.0, 101.0, 99.0, 100.5)]),
                ("BAD".into(), vec![]),
            ],
            last_calls: Mutex::new(Vec::new()),
        };
        let cfg = MarketHistoryConfig {
            symbols: vec!["SPY".into(), "BAD".into()],
            interval: "1d".into(),
            range: "3mo".into(),
        };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let series = parsed.pointer("/data/symbols").unwrap().as_array().unwrap();
        // Only SPY made it through; BAD was skipped.
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].get("symbol").unwrap().as_str().unwrap(), "SPY",);
    }

    #[tokio::test]
    async fn run_cycle_all_failures_returns_upstream_error() {
        let pool = open_in_memory().await.unwrap();
        let cfg = MarketHistoryConfig::default();
        let err = run_cycle(&pool, &FailingFetcher, &cfg).await.unwrap_err();
        assert!(
            matches!(err, MarketsSeederError::Upstream(_)),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn run_cycle_all_empty_returns_empty_upstream_error() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows_by_symbol: vec![],
            last_calls: Mutex::new(Vec::new()),
        };
        let cfg = MarketHistoryConfig {
            symbols: vec!["A".into(), "B".into()],
            interval: "1d".into(),
            range: "3mo".into(),
        };
        let err = run_cycle(&pool, &fetcher, &cfg).await.unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta_with_cascade_group() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows_by_symbol: vec![("SPY".into(), vec![bar(1, 100.0, 101.0, 99.0, 100.5)])],
            last_calls: Mutex::new(Vec::new()),
        };
        let cfg = MarketHistoryConfig {
            symbols: vec!["SPY".into()],
            interval: "1d".into(),
            range: "3mo".into(),
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
