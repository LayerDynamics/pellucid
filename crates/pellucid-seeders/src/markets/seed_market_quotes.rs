//! seed_market_quotes — FAST-tier snapshot of broad-market
//! ETF/index proxies (SPEC-001 §17.7).
//!
//! The webview's `MarketsPanel` renders a dashboard tile per
//! symbol; the snapshot ships everything in one cache entry.
//!
//! Symbols (default basket):
//! - `SPY` — S&P 500 ETF
//! - `QQQ` — Nasdaq-100 ETF
//! - `DIA` — Dow Jones Industrial Average ETF
//! - `IWM` — Russell 2000 ETF
//! - `VTI` — Total US Stock Market ETF
//! - `EFA` — MSCI EAFE (developed ex-US) ETF
//! - `EEM` — MSCI Emerging Markets ETF
//! - `^VIX` — CBOE Volatility Index
//!
//! These cover the same surface area the original WorldMonitor
//! `markets` seeder did. The basket is configurable via
//! [`MarketQuotesConfig::symbols`] so test rigs can supply a
//! shorter list to keep wiremock fixtures small.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::MarketsSeederError;

/// Cache key — written to FAST tier. Mirrors the existing
/// `FAST_KEYS` slot in `pellucid_handlers::bootstrap::keys`.
pub const CACHE_KEY: &str = "market:stocks-bootstrap:v1";

/// FAST-tier TTL — 60s `s-maxage` on the gateway side.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp the seed_meta carries.
pub const SOURCE_VERSION: &str = "market-quotes-yahoo-v1";

/// Cascade group tag — the panel + this seeder share it.
pub const CASCADE_GROUP: &str = "markets";

/// Default basket — the eight symbols listed above.
pub const DEFAULT_SYMBOLS: &[&str] = &["SPY", "QQQ", "DIA", "IWM", "VTI", "EFA", "EEM", "^VIX"];

/// Run-time configuration for the seeder.
#[derive(Clone, Debug)]
pub struct MarketQuotesConfig {
    /// Symbols to fetch. Production uses [`DEFAULT_SYMBOLS`];
    /// tests pass a shorter slice to keep wiremock fixtures
    /// minimal.
    pub symbols: Vec<String>,
}

impl Default for MarketQuotesConfig {
    fn default() -> Self {
        Self {
            symbols: DEFAULT_SYMBOLS.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// One per-symbol row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketQuoteRow {
    /// Ticker symbol (echoed from the request).
    pub symbol: String,
    /// Most-recent regular-session price.
    pub price: f64,
    /// Previous-session close.
    pub previous_close: f64,
    /// Percent change vs `previous_close`. Pre-computed so the
    /// panel doesn't have to.
    pub percent_change: f64,
    /// Currency the price is quoted in.
    pub currency: String,
    /// Exchange code (e.g. `PCX` for SPY, `NYQ` for VIX).
    pub exchange: String,
    /// Wall-clock seconds when Yahoo stamped this row.
    pub regular_market_time: i64,
}

/// Published snapshot — exactly the shape `MarketsPanel` reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketQuotesSnapshot {
    /// One row per requested symbol, in input order. Symbols
    /// the upstream had no data for are omitted (the panel
    /// dim-renders them).
    pub rows: Vec<MarketQuoteRow>,
    /// Wall-clock ms when the snapshot was assembled. Drives
    /// the panel's "as of" footer.
    pub assembled_at_ms: i64,
}

/// DI trait — implemented by an adapter that wraps
/// [`pellucid_streams::YahooFinanceClient`]. Keeps `pellucid-seeders`
/// from taking a hard dep on `pellucid-streams` in its own
/// dependency graph.
#[async_trait]
pub trait MarketQuotesFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch quotes for `symbols`. Returns one entry per input
    /// symbol in the same order — `None` for symbols the
    /// upstream had no data for. Concrete adapters delegate to
    /// `YahooFinanceClient::fetch_quotes`.
    async fn fetch_quotes(
        &self,
        symbols: &[&str],
    ) -> Result<Vec<Option<FetchedQuote>>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Distilled quote shape the trait surfaces. Mirrors the
/// fields `MarketQuoteRow` needs without leaking the
/// `pellucid_streams::YahooQuote` type into this crate's
/// public API.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedQuote {
    /// Ticker symbol as the upstream returned it.
    pub symbol: String,
    /// Most-recent regular-market price.
    pub price: f64,
    /// Previous-session close.
    pub previous_close: f64,
    /// Currency code (`USD`, `EUR`, …).
    pub currency: String,
    /// Exchange code.
    pub exchange: String,
    /// Wall-clock seconds when the upstream stamped this row.
    pub regular_market_time: i64,
}

impl FetchedQuote {
    /// Percent change relative to `previous_close`. 0.0 when
    /// `previous_close` is 0 (freshly listed symbols).
    #[must_use]
    pub fn percent_change(&self) -> f64 {
        if self.previous_close == 0.0 {
            0.0
        } else {
            (self.price - self.previous_close) / self.previous_close * 100.0
        }
    }
}

/// Run one cycle: fetch every symbol, assemble snapshot,
/// atomic-publish.
///
/// # Errors
/// See [`MarketsSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn MarketQuotesFetcher,
    config: &MarketQuotesConfig,
) -> Result<PublishOutcome, MarketsSeederError> {
    let symbol_refs: Vec<&str> = config.symbols.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_quotes(&symbol_refs)
        .await
        .map_err(|e| MarketsSeederError::Upstream(e.to_string()))?;

    let mut rows: Vec<MarketQuoteRow> = Vec::with_capacity(fetched.len());
    for q in fetched.into_iter().flatten() {
        rows.push(MarketQuoteRow {
            percent_change: q.percent_change(),
            symbol: q.symbol,
            price: q.price,
            previous_close: q.previous_close,
            currency: q.currency,
            exchange: q.exchange,
            regular_market_time: q.regular_market_time,
        });
    }
    if rows.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = MarketQuotesSnapshot {
        rows,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(60_000),
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

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<Option<FetchedQuote>>,
    }

    #[async_trait]
    impl MarketQuotesFetcher for StaticFetcher {
        async fn fetch_quotes(
            &self,
            _symbols: &[&str],
        ) -> Result<Vec<Option<FetchedQuote>>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl MarketQuotesFetcher for FailingFetcher {
        async fn fetch_quotes(
            &self,
            _symbols: &[&str],
        ) -> Result<Vec<Option<FetchedQuote>>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn quote(symbol: &str, price: f64, prev: f64) -> FetchedQuote {
        FetchedQuote {
            symbol: symbol.to_string(),
            price,
            previous_close: prev,
            currency: "USD".into(),
            exchange: "PCX".into(),
            regular_market_time: 1_714_060_800,
        }
    }

    #[test]
    fn cache_key_matches_fast_keys_slot() {
        assert_eq!(CACHE_KEY, "market:stocks-bootstrap:v1");
    }

    #[test]
    fn ttl_is_fast_tier() {
        assert_eq!(TTL, Duration::from_secs(60));
    }

    #[test]
    fn default_symbols_cover_eight_baskets() {
        let cfg = MarketQuotesConfig::default();
        assert_eq!(cfg.symbols.len(), 8);
        for sym in ["SPY", "QQQ", "DIA", "IWM", "VTI", "EFA", "EEM", "^VIX"] {
            assert!(cfg.symbols.iter().any(|s| s == sym), "missing {sym}");
        }
    }

    #[test]
    fn fetched_quote_percent_change_zero_previous_is_zero() {
        let q = quote("X", 100.0, 0.0);
        assert!((q.percent_change() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn fetched_quote_percent_change_simple() {
        let q = quote("X", 110.0, 100.0);
        assert!((q.percent_change() - 10.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_one_row_per_quote() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                Some(quote("SPY", 524.0, 522.0)),
                Some(quote("QQQ", 460.0, 458.0)),
                None,
                Some(quote("DIA", 390.0, 389.0)),
            ],
        };
        let outcome = run_cycle(
            &pool,
            &fetcher,
            &MarketQuotesConfig {
                symbols: vec!["SPY".into(), "QQQ".into(), "BAD".into(), "DIA".into()],
            },
        )
        .await
        .unwrap();
        assert!(outcome.bytes_written > 0);

        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 3);
        let symbols: Vec<_> = rows
            .iter()
            .map(|r| r.get("symbol").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(symbols, vec!["SPY", "QQQ", "DIA"]);
        let percent = rows[0].get("percent_change").unwrap().as_f64().unwrap();
        assert!((percent - ((524.0 - 522.0) / 522.0 * 100.0)).abs() < 1e-9);
    }

    #[tokio::test]
    async fn run_cycle_empty_results_returns_empty_upstream_error() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![None, None],
        };
        let err = run_cycle(
            &pool,
            &fetcher,
            &MarketQuotesConfig {
                symbols: vec!["A".into(), "B".into()],
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, MarketsSeederError::EmptyUpstream),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &MarketQuotesConfig::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, MarketsSeederError::Upstream(_)),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta_with_cascade_group() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![Some(quote("SPY", 524.0, 522.0))],
        };
        let _ = run_cycle(
            &pool,
            &fetcher,
            &MarketQuotesConfig {
                symbols: vec!["SPY".into()],
            },
        )
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
