//! seed_stablecoin_quotes — FAST-tier snapshot of major
//! stablecoin prices + peg deviation.
//!
//! The webview's `StablecoinPanel` (T4.2.10) renders a compact
//! grid of stablecoin price + 24 h change + market cap + a peg-
//! deviation badge (`>1%` deviation from $1.00 = "depeg risk").
//!
//! Default basket — the four highest-cap USD stablecoins by
//! late-2025 ranking; the production binary can override the
//! list at boot.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::MarketsSeederError;

/// Cache key — FAST tier.
pub const CACHE_KEY: &str = "market:stablecoin-snapshot:v1";

/// FAST-tier TTL — 60 s.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "stablecoin-coingecko-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "markets-crypto";

/// Default basket of CoinGecko ids — the four most-traded USD
/// stablecoins.
pub const DEFAULT_IDS: &[&str] = &[
    "tether",
    "usd-coin",
    "dai",
    "first-digital-usd",
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct StablecoinQuotesConfig {
    /// CoinGecko ids — `tether`, `usd-coin`, etc.
    pub ids: Vec<String>,
}

impl Default for StablecoinQuotesConfig {
    fn default() -> Self {
        Self {
            ids: DEFAULT_IDS.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// One row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StablecoinQuoteRow {
    /// CoinGecko id.
    pub id: String,
    /// Display ticker (uppercase).
    pub symbol: String,
    /// USD price.
    pub usd: f64,
    /// 24-hour percent change vs USD.
    pub usd_24h_change: f64,
    /// USD market cap. 0.0 when CoinGecko omits it.
    pub usd_market_cap: f64,
    /// Peg deviation in percent (signed; 0 means "exactly $1.00").
    /// Pre-computed by the seeder so the panel doesn't have to.
    pub peg_deviation_pct: f64,
    /// Wall-clock seconds when CoinGecko stamped this row.
    pub last_updated_at: i64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StablecoinQuotesSnapshot {
    /// One row per id the upstream returned (missing ids are
    /// silently dropped).
    pub rows: Vec<StablecoinQuoteRow>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled quote — mirrors `pellucid_streams::CryptoQuote`
/// shape without leaking the streams type.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedStablecoinQuote {
    /// CoinGecko id.
    pub id: String,
    /// Display ticker.
    pub symbol: String,
    /// USD price.
    pub usd: f64,
    /// 24h change.
    pub usd_24h_change: f64,
    /// USD market cap.
    pub usd_market_cap: f64,
    /// Last-update wall-clock seconds.
    pub last_updated_at: i64,
}

/// DI trait — wraps `pellucid_streams::CoinGeckoClient::fetch_simple_price`.
#[async_trait]
pub trait StablecoinFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch USD-denominated stablecoin rows.
    async fn fetch_stablecoins(
        &self,
        ids: &[&str],
    ) -> Result<Vec<FetchedStablecoinQuote>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Compute peg deviation in percent. Pure.
#[must_use]
pub fn peg_deviation(usd: f64) -> f64 {
    if !usd.is_finite() || usd <= 0.0 {
        return 0.0;
    }
    (usd - 1.0) * 100.0
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn StablecoinFetcher,
    config: &StablecoinQuotesConfig,
) -> Result<PublishOutcome, MarketsSeederError> {
    let id_refs: Vec<&str> = config.ids.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_stablecoins(&id_refs)
        .await
        .map_err(|e| MarketsSeederError::Upstream(e.to_string()))?;

    if fetched.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let rows: Vec<StablecoinQuoteRow> = fetched
        .into_iter()
        .map(|q| StablecoinQuoteRow {
            peg_deviation_pct: peg_deviation(q.usd),
            id: q.id,
            symbol: q.symbol.to_uppercase(),
            usd: q.usd,
            usd_24h_change: q.usd_24h_change,
            usd_market_cap: q.usd_market_cap,
            last_updated_at: q.last_updated_at,
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = StablecoinQuotesSnapshot {
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
        rows: Vec<FetchedStablecoinQuote>,
    }

    #[async_trait]
    impl StablecoinFetcher for StaticFetcher {
        async fn fetch_stablecoins(
            &self,
            _ids: &[&str],
        ) -> Result<Vec<FetchedStablecoinQuote>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    fn quote(id: &str, sym: &str, usd: f64) -> FetchedStablecoinQuote {
        FetchedStablecoinQuote {
            id: id.into(),
            symbol: sym.into(),
            usd,
            usd_24h_change: 0.0,
            usd_market_cap: 1_000_000_000.0,
            last_updated_at: 1_714_060_800,
        }
    }

    #[test]
    fn peg_deviation_centers_on_dollar() {
        assert!((peg_deviation(1.00) - 0.0).abs() < 1e-9);
        assert!((peg_deviation(1.01) - 1.0).abs() < 1e-9);
        assert!((peg_deviation(0.99) - -1.0).abs() < 1e-9);
        // Non-finite or non-positive returns 0 (would otherwise
        // emit a misleading huge negative deviation).
        assert_eq!(peg_deviation(0.0), 0.0);
        assert_eq!(peg_deviation(-1.0), 0.0);
        assert_eq!(peg_deviation(f64::NAN), 0.0);
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "market:stablecoin-snapshot:v1");
    }

    #[test]
    fn default_basket_has_four_stablecoins() {
        assert_eq!(StablecoinQuotesConfig::default().ids.len(), 4);
    }

    #[tokio::test]
    async fn run_cycle_writes_uppercased_symbols_and_pre_computes_peg_deviation() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                quote("tether", "usdt", 1.00),
                quote("usd-coin", "usdc", 0.998),
                quote("dai", "dai", 1.005),
            ],
        };
        let _ = run_cycle(&pool, &fetcher, &StablecoinQuotesConfig::default())
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as(
            "SELECT payload FROM kv_envelope WHERE cache_key = ?",
        )
        .bind(CACHE_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        let symbols: Vec<&str> = rows
            .iter()
            .map(|r| r.get("symbol").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(symbols, vec!["USDT", "USDC", "DAI"]);
        let usdt_pct = rows[0]
            .get("peg_deviation_pct")
            .unwrap()
            .as_f64()
            .unwrap();
        assert!((usdt_pct - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream_error() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &StablecoinQuotesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }
}
