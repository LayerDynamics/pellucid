//! seed_commodity_quotes — FAST-tier snapshot of commodity
//! continuous-front-month futures (SPEC-001 §17.7).
//!
//! Yahoo Finance ticks continuous-contract futures with a `=F`
//! suffix. The default basket targets the eight commodities the
//! webview's `CommoditiesPanel` renders:
//!
//! | Symbol | Commodity              | Exchange |
//! |--------|------------------------|----------|
//! | `CL=F` | WTI Crude Oil          | NYMEX    |
//! | `BZ=F` | Brent Crude Oil        | ICE      |
//! | `NG=F` | Henry Hub Natural Gas  | NYMEX    |
//! | `GC=F` | Gold                   | COMEX    |
//! | `SI=F` | Silver                 | COMEX    |
//! | `HG=F` | Copper                 | COMEX    |
//! | `ZW=F` | Wheat                  | CBOT     |
//! | `ZC=F` | Corn                   | CBOT     |
//!
//! Same DI-trait pattern as `seed_market_quotes` — production
//! adapter wraps `pellucid_streams::YahooFinanceClient`, tests
//! pass a static fetcher.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::seed_market_quotes::{FetchedQuote, MarketQuotesFetcher};
use crate::markets::MarketsSeederError;

/// Cache key — FAST tier slot in `pellucid_handlers::bootstrap::keys`.
pub const CACHE_KEY: &str = "market:commodities-snapshot:v1";

/// FAST-tier TTL (60s s-maxage on the gateway).
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "commodity-quotes-yahoo-v1";

/// Cascade group tag — shared with the markets domain.
pub const CASCADE_GROUP: &str = "markets-commodities";

/// Default basket — the eight continuous-contract symbols.
pub const DEFAULT_SYMBOLS: &[&str] = &[
    "CL=F", "BZ=F", "NG=F", "GC=F", "SI=F", "HG=F", "ZW=F", "ZC=F",
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct CommodityQuotesConfig {
    /// Yahoo continuous-front-month tickers (`<root>=F`).
    pub symbols: Vec<String>,
}

impl Default for CommodityQuotesConfig {
    fn default() -> Self {
        Self {
            symbols: DEFAULT_SYMBOLS.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// One commodity row in the snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommodityQuoteRow {
    /// Yahoo continuous-front-month ticker (e.g. `CL=F`).
    pub symbol: String,
    /// Most-recent regular-session price.
    pub price: f64,
    /// Previous-session settlement.
    pub previous_close: f64,
    /// Pre-computed percent change vs `previous_close`.
    pub percent_change: f64,
    /// Pricing currency (USD for the default basket).
    pub currency: String,
    /// Exchange code (`NYM`, `CMX`, `CBT`).
    pub exchange: String,
    /// Wall-clock seconds when the upstream stamped this row.
    pub regular_market_time: i64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommodityQuotesSnapshot {
    /// One row per upstream-known symbol, in input order.
    pub rows: Vec<CommodityQuoteRow>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Run one cycle.
///
/// # Errors
/// See [`MarketsSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn MarketQuotesFetcher,
    config: &CommodityQuotesConfig,
) -> Result<PublishOutcome, MarketsSeederError> {
    let symbol_refs: Vec<&str> = config.symbols.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_quotes(&symbol_refs)
        .await
        .map_err(|e| MarketsSeederError::Upstream(e.to_string()))?;

    let mut rows: Vec<CommodityQuoteRow> = Vec::with_capacity(fetched.len());
    for q in fetched.into_iter().flatten() {
        rows.push(map_row(q));
    }
    if rows.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = CommodityQuotesSnapshot {
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

fn map_row(q: FetchedQuote) -> CommodityQuoteRow {
    CommodityQuoteRow {
        percent_change: q.percent_change(),
        symbol: q.symbol,
        price: q.price,
        previous_close: q.previous_close,
        currency: q.currency,
        exchange: q.exchange,
        regular_market_time: q.regular_market_time,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
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

    fn quote(symbol: &str, price: f64, prev: f64) -> FetchedQuote {
        FetchedQuote {
            symbol: symbol.to_string(),
            price,
            previous_close: prev,
            currency: "USD".into(),
            exchange: "NYM".into(),
            regular_market_time: 1_714_060_800,
        }
    }

    #[test]
    fn cache_key_matches_fast_keys_slot() {
        assert_eq!(CACHE_KEY, "market:commodities-snapshot:v1");
    }

    #[test]
    fn default_symbols_include_eight_majors() {
        let cfg = CommodityQuotesConfig::default();
        assert_eq!(cfg.symbols.len(), 8);
        for sym in DEFAULT_SYMBOLS {
            assert!(cfg.symbols.iter().any(|s| s == sym), "missing {sym}");
        }
    }

    #[test]
    fn map_row_pre_computes_percent_change() {
        let row = map_row(quote("CL=F", 82.0, 80.0));
        assert!((row.percent_change - ((82.0 - 80.0) / 80.0 * 100.0)).abs() < 1e-9);
        assert_eq!(row.symbol, "CL=F");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                Some(quote("CL=F", 82.0, 80.0)),
                Some(quote("GC=F", 2350.0, 2340.0)),
                None,
                Some(quote("NG=F", 2.10, 2.05)),
            ],
        };
        let outcome = run_cycle(
            &pool,
            &fetcher,
            &CommodityQuotesConfig {
                symbols: vec!["CL=F".into(), "GC=F".into(), "BAD=F".into(), "NG=F".into()],
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
        assert_eq!(rows[0].get("symbol").unwrap().as_str().unwrap(), "CL=F");
    }

    #[tokio::test]
    async fn run_cycle_empty_results_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![None, None],
        };
        let err = run_cycle(
            &pool,
            &fetcher,
            &CommodityQuotesConfig {
                symbols: vec!["A=F".into(), "B=F".into()],
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![Some(quote("CL=F", 82.0, 80.0))],
        };
        let _ = run_cycle(
            &pool,
            &fetcher,
            &CommodityQuotesConfig {
                symbols: vec!["CL=F".into()],
            },
        )
        .await
        .unwrap();
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
