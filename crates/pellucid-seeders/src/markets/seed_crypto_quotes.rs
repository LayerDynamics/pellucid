//! seed_crypto_quotes — FAST-tier snapshot of major crypto
//! tokens via CoinGecko (SPEC-001 §17.7).
//!
//! Default basket — the eight tokens the webview's
//! `CryptoPanel` renders:
//!
//! | CoinGecko id    | Symbol | Name           |
//! |-----------------|--------|----------------|
//! | `bitcoin`       | BTC    | Bitcoin        |
//! | `ethereum`      | ETH    | Ethereum       |
//! | `solana`        | SOL    | Solana         |
//! | `ripple`        | XRP    | XRP            |
//! | `cardano`       | ADA    | Cardano        |
//! | `avalanche-2`   | AVAX   | Avalanche      |
//! | `dogecoin`      | DOGE   | Dogecoin       |
//! | `chainlink`     | LINK   | Chainlink      |
//!
//! Same DI-trait pattern. Production adapter wraps
//! `pellucid_streams::CoinGeckoClient`.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::MarketsSeederError;

/// Cache key — FAST-tier slot newly added to
/// `pellucid_handlers::bootstrap::keys::FAST_KEYS` for T3.8.
pub const CACHE_KEY: &str = "market:crypto-snapshot:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "crypto-quotes-coingecko-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "markets-crypto";

/// Default basket of CoinGecko ids.
pub const DEFAULT_IDS: &[&str] = &[
    "bitcoin",
    "ethereum",
    "solana",
    "ripple",
    "cardano",
    "avalanche-2",
    "dogecoin",
    "chainlink",
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct CryptoQuotesConfig {
    /// CoinGecko ids — `bitcoin`, `ethereum`, etc.
    pub ids: Vec<String>,
}

impl Default for CryptoQuotesConfig {
    fn default() -> Self {
        Self {
            ids: DEFAULT_IDS.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// One row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoQuoteRow {
    /// CoinGecko id (e.g. `"bitcoin"`).
    pub id: String,
    /// USD price.
    pub usd: f64,
    /// 24-hour percent change vs USD (signed).
    pub usd_24h_change: f64,
    /// USD market cap. 0.0 when CoinGecko omits it.
    pub usd_market_cap: f64,
    /// Wall-clock seconds when CoinGecko stamped this row.
    pub last_updated_at: i64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoQuotesSnapshot {
    /// One row per id the upstream returned (missing ids are
    /// silently dropped — CoinGecko's own contract).
    pub rows: Vec<CryptoQuoteRow>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// DI trait — wraps `pellucid_streams::CoinGeckoClient::fetch_simple_price`.
#[async_trait]
pub trait CryptoQuotesFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch USD-denominated rows for `ids`. Missing ids are
    /// dropped (matches the upstream's behaviour).
    async fn fetch_simple_price(
        &self,
        ids: &[&str],
    ) -> Result<Vec<FetchedCryptoQuote>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Distilled crypto quote — mirrors `pellucid_streams::CryptoQuote`
/// without leaking the type into this crate's public API.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedCryptoQuote {
    /// CoinGecko id.
    pub id: String,
    /// USD price.
    pub usd: f64,
    /// 24-hour percent change.
    pub usd_24h_change: f64,
    /// Market cap in USD.
    pub usd_market_cap: f64,
    /// Wall-clock seconds the upstream stamped this row.
    pub last_updated_at: i64,
}

/// Run one cycle.
///
/// # Errors
/// See [`MarketsSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn CryptoQuotesFetcher,
    config: &CryptoQuotesConfig,
) -> Result<PublishOutcome, MarketsSeederError> {
    let id_refs: Vec<&str> = config.ids.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_simple_price(&id_refs)
        .await
        .map_err(|e| MarketsSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let rows: Vec<CryptoQuoteRow> = fetched
        .into_iter()
        .map(|q| CryptoQuoteRow {
            id: q.id,
            usd: q.usd,
            usd_24h_change: q.usd_24h_change,
            usd_market_cap: q.usd_market_cap,
            last_updated_at: q.last_updated_at,
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = CryptoQuotesSnapshot {
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
        rows: Vec<FetchedCryptoQuote>,
    }

    #[async_trait]
    impl CryptoQuotesFetcher for StaticFetcher {
        async fn fetch_simple_price(
            &self,
            _ids: &[&str],
        ) -> Result<Vec<FetchedCryptoQuote>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl CryptoQuotesFetcher for FailingFetcher {
        async fn fetch_simple_price(
            &self,
            _ids: &[&str],
        ) -> Result<Vec<FetchedCryptoQuote>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn quote(id: &str, usd: f64) -> FetchedCryptoQuote {
        FetchedCryptoQuote {
            id: id.to_string(),
            usd,
            usd_24h_change: 1.5,
            usd_market_cap: 1.0e12,
            last_updated_at: 1_714_060_800,
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "market:crypto-snapshot:v1");
    }

    #[test]
    fn default_ids_cover_eight_tokens() {
        let cfg = CryptoQuotesConfig::default();
        assert_eq!(cfg.ids.len(), 8);
        for id in DEFAULT_IDS {
            assert!(cfg.ids.iter().any(|s| s == id), "missing {id}");
        }
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![quote("bitcoin", 67_234.0), quote("ethereum", 3450.0)],
        };
        let outcome = run_cycle(&pool, &fetcher, &CryptoQuotesConfig::default())
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
        assert_eq!(rows[0].get("id").unwrap().as_str().unwrap(), "bitcoin");
    }

    #[tokio::test]
    async fn run_cycle_empty_upstream_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &CryptoQuotesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_error_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &CryptoQuotesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![quote("bitcoin", 67_234.0)],
        };
        let _ = run_cycle(&pool, &fetcher, &CryptoQuotesConfig::default())
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
