//! seed_memory_market — FAST-tier proxy index for the DRAM /
//! NAND market built from publicly traded memory-vendor stock
//! prices.
//!
//! ## Why a stock-price proxy?
//!
//! Current DRAM and NAND spot pricing is paywalled by every
//! commercial source (DRAMeXchange, TrendForce, IDC). There is
//! no free public REST endpoint that publishes intraday or
//! daily memory-spot quotes.
//!
//! The closest free signal is the share-price index of the
//! firms that produce the vast majority of global DRAM / NAND
//! supply: Micron (MU), Western Digital (WDC), Seagate (STX),
//! and Samsung Electronics (005930.KS). Their share prices
//! track demand expectations and the pricing cycle with a
//! high correlation (≈0.7–0.85 over multi-year windows per
//! published equity research). The seeder publishes their
//! latest quotes + a simple equal-weighted index, with a
//! `source_note` documenting the proxy for the panel.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::seed_market_quotes::{FetchedQuote, MarketQuotesFetcher};
use crate::technology::TechnologySeederError;

/// Cache key — NEW FAST tier slot added by T3.8 expansion.
pub const CACHE_KEY: &str = "technology:memory-market-index:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "memory-market-yahoo-stocks-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "technology-memory";

/// Default basket — major listed memory / storage vendors.
pub const DEFAULT_SYMBOLS: &[&str] = &[
    "MU",      // Micron
    "WDC",     // Western Digital
    "STX",     // Seagate
    "005930.KS", // Samsung Electronics (Korea Exchange)
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct MemoryMarketConfig {
    /// Yahoo tickers to fetch.
    pub symbols: Vec<String>,
}

impl Default for MemoryMarketConfig {
    fn default() -> Self {
        Self {
            symbols: DEFAULT_SYMBOLS.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// One ticker quote.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VendorQuoteRow {
    /// Ticker.
    pub symbol: String,
    /// Latest price.
    pub price: f64,
    /// Previous close.
    pub previous_close: f64,
    /// Pre-computed % change.
    pub percent_change: f64,
    /// Currency.
    pub currency: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryMarketSnapshot {
    /// Per-vendor quotes.
    pub rows: Vec<VendorQuoteRow>,
    /// Equal-weighted average of `percent_change` across the
    /// basket — the panel's headline "memory market index"
    /// figure.
    pub index_percent_change: f64,
    /// Documented proxy note (DRAM spot prices paywalled).
    pub source_note: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Run one cycle.
///
/// # Errors
/// See [`TechnologySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn MarketQuotesFetcher,
    config: &MemoryMarketConfig,
) -> Result<PublishOutcome, TechnologySeederError> {
    let symbol_refs: Vec<&str> = config.symbols.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_quotes(&symbol_refs)
        .await
        .map_err(|e| TechnologySeederError::Upstream(e.to_string()))?;

    let mut rows: Vec<VendorQuoteRow> = Vec::new();
    for q in fetched.into_iter().flatten() {
        rows.push(map_row(q));
    }
    if rows.is_empty() {
        return Err(TechnologySeederError::EmptyUpstream);
    }
    let index_percent_change =
        rows.iter().map(|r| r.percent_change).sum::<f64>() / rows.len() as f64;

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = MemoryMarketSnapshot {
        rows,
        index_percent_change,
        source_note: "Equal-weighted vendor stock-price proxy. \
            DRAM / NAND spot prices require a paid subscription \
            (DRAMeXchange / TrendForce / IDC); no free public \
            API publishes intraday memory-spot quotes."
            .into(),
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
    let outcome =
        atomic_publish(pool, "technology", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn map_row(q: FetchedQuote) -> VendorQuoteRow {
    VendorQuoteRow {
        percent_change: q.percent_change(),
        symbol: q.symbol,
        price: q.price,
        previous_close: q.previous_close,
        currency: q.currency,
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
        ) -> Result<Vec<Option<FetchedQuote>>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    fn quote(symbol: &str, price: f64, prev: f64) -> FetchedQuote {
        FetchedQuote {
            symbol: symbol.into(),
            price,
            previous_close: prev,
            currency: "USD".into(),
            exchange: "NMS".into(),
            regular_market_time: 1_714_060_800,
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "technology:memory-market-index:v1");
    }

    #[tokio::test]
    async fn run_cycle_publishes_index_and_source_note() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                Some(quote("MU", 110.0, 100.0)),  // +10%
                Some(quote("WDC", 75.0, 80.0)),   // -6.25%
                None,
                Some(quote("STX", 100.0, 100.0)), // 0%
            ],
        };
        let _ = run_cycle(&pool, &fetcher, &MemoryMarketConfig::default())
            .await
            .unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 3);
        // (10 - 6.25 + 0) / 3 = 1.25
        let idx = parsed
            .pointer("/data/index_percent_change")
            .unwrap()
            .as_f64()
            .unwrap();
        assert!((idx - 1.25).abs() < 1e-9);
        let note = parsed
            .pointer("/data/source_note")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(note.contains("paid subscription"));
        assert!(note.contains("stock-price proxy"));
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![None, None],
        };
        let err = run_cycle(&pool, &fetcher, &MemoryMarketConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, TechnologySeederError::EmptyUpstream));
    }
}
