//! seed_cot — SLOW-tier weekly Commitments of Traders snapshot
//! (SPEC-001 §17.7).
//!
//! The CFTC publishes the Disaggregated COT report every
//! Friday around 15:30 ET. The Pellucid `CotPanel` renders one
//! row per watched contract showing
//! `(producer_long, producer_short, swap_long, swap_short,
//! managed_money_long, managed_money_short)` with a managed-
//! money net column.
//!
//! Default contracts (CFTC contract market codes):
//!
//! | Code     | Contract                            |
//! |----------|-------------------------------------|
//! | `088691` | Gold (COMEX)                        |
//! | `084691` | Silver (COMEX)                      |
//! | `085692` | Copper (COMEX)                      |
//! | `067411` | Crude Oil, Light Sweet (NYMEX)      |
//! | `023391` | Wheat (CBOT)                        |
//! | `002602` | Corn (CBOT)                         |
//! | `005602` | Soybeans (CBOT)                     |
//! | `023651` | Live Cattle (CME)                   |
//!
//! Production adapter wraps `pellucid_streams::CftcCotClient`.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::MarketsSeederError;

/// Cache key — SLOW-tier slot newly added to
/// `pellucid_handlers::bootstrap::keys::SLOW_KEYS`.
pub const CACHE_KEY: &str = "market:cot-report:weekly:v1";

/// SLOW-tier TTL — 24 hours. The report only refreshes weekly,
/// so a 24-hour TTL is safely conservative; the seeder's own
/// scheduling cadence is daily.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "cot-cftc-disaggregated-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "markets-cot";

/// Default basket of CFTC contract market codes.
pub const DEFAULT_CONTRACT_CODES: &[&str] = &[
    "088691", // Gold (COMEX)
    "084691", // Silver (COMEX)
    "085692", // Copper (COMEX)
    "067411", // Crude Oil Light Sweet (NYMEX)
    "023391", // Wheat (CBOT)
    "002602", // Corn (CBOT)
    "005602", // Soybeans (CBOT)
    "023651", // Live Cattle (CME)
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct CotConfig {
    /// CFTC contract market codes to fetch.
    pub contract_codes: Vec<String>,
}

impl Default for CotConfig {
    fn default() -> Self {
        Self {
            contract_codes: DEFAULT_CONTRACT_CODES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

/// One per-contract row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CotPublishedRow {
    /// CFTC contract market code (e.g. `"088691"`).
    pub contract_code: String,
    /// Human-readable contract name (e.g. `"GOLD"`).
    pub contract_name: String,
    /// Report week (`YYYY-MM-DD`).
    pub report_date: String,
    /// Aggregate open interest across all categories.
    pub open_interest_all: i64,
    /// Producer/merchant/processor/user long.
    pub producer_long: i64,
    /// Producer/merchant/processor/user short.
    pub producer_short: i64,
    /// Swap dealer long.
    pub swap_long: i64,
    /// Swap dealer short.
    pub swap_short: i64,
    /// Managed-money long.
    pub managed_money_long: i64,
    /// Managed-money short.
    pub managed_money_short: i64,
    /// Pre-computed `managed_money_long − managed_money_short`.
    /// Positive → net long (bullish), negative → net short.
    pub managed_money_net: i64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CotSnapshot {
    /// One row per contract code the upstream returned.
    pub rows: Vec<CotPublishedRow>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled COT row — mirrors `pellucid_streams::CotRow`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedCotRow {
    /// CFTC contract market code.
    pub contract_code: String,
    /// Contract name.
    pub contract_name: String,
    /// Report week.
    pub report_date: String,
    /// Open interest.
    pub open_interest_all: i64,
    /// Producer long.
    pub producer_long: i64,
    /// Producer short.
    pub producer_short: i64,
    /// Swap long.
    pub swap_long: i64,
    /// Swap short.
    pub swap_short: i64,
    /// Managed-money long.
    pub managed_money_long: i64,
    /// Managed-money short.
    pub managed_money_short: i64,
}

impl FetchedCotRow {
    /// Net managed-money position (long − short).
    #[must_use]
    pub const fn managed_money_net(&self) -> i64 {
        self.managed_money_long - self.managed_money_short
    }
}

/// DI trait — wraps `pellucid_streams::CftcCotClient::fetch_disaggregated`.
#[async_trait]
pub trait CotFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest Disaggregated COT row per contract code.
    async fn fetch_disaggregated(
        &self,
        contract_codes: &[&str],
    ) -> Result<Vec<FetchedCotRow>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`MarketsSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn CotFetcher,
    config: &CotConfig,
) -> Result<PublishOutcome, MarketsSeederError> {
    let code_refs: Vec<&str> = config.contract_codes.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_disaggregated(&code_refs)
        .await
        .map_err(|e| MarketsSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let rows: Vec<CotPublishedRow> = fetched
        .into_iter()
        .map(|r| CotPublishedRow {
            managed_money_net: r.managed_money_net(),
            contract_code: r.contract_code,
            contract_name: r.contract_name,
            report_date: r.report_date,
            open_interest_all: r.open_interest_all,
            producer_long: r.producer_long,
            producer_short: r.producer_short,
            swap_long: r.swap_long,
            swap_short: r.swap_short,
            managed_money_long: r.managed_money_long,
            managed_money_short: r.managed_money_short,
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = CotSnapshot {
        rows,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(86_400_000),
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
        rows: Vec<FetchedCotRow>,
    }

    #[async_trait]
    impl CotFetcher for StaticFetcher {
        async fn fetch_disaggregated(
            &self,
            _codes: &[&str],
        ) -> Result<Vec<FetchedCotRow>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl CotFetcher for FailingFetcher {
        async fn fetch_disaggregated(
            &self,
            _codes: &[&str],
        ) -> Result<Vec<FetchedCotRow>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn row(code: &str, name: &str, mm_long: i64, mm_short: i64) -> FetchedCotRow {
        FetchedCotRow {
            contract_code: code.into(),
            contract_name: name.into(),
            report_date: "2026-04-25".into(),
            open_interest_all: 500_000,
            producer_long: 60_000,
            producer_short: 70_000,
            swap_long: 150_000,
            swap_short: 30_000,
            managed_money_long: mm_long,
            managed_money_short: mm_short,
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "market:cot-report:weekly:v1");
    }

    #[test]
    fn ttl_is_one_day() {
        assert_eq!(TTL, Duration::from_secs(24 * 60 * 60));
    }

    #[test]
    fn default_basket_includes_gold_silver_crude() {
        let cfg = CotConfig::default();
        for code in ["088691", "084691", "067411"] {
            assert!(
                cfg.contract_codes.iter().any(|c| c == code),
                "missing {code}"
            );
        }
    }

    #[test]
    fn fetched_row_managed_money_net_signs_correctly() {
        let r = row("X", "X", 120_000, 30_000);
        assert_eq!(r.managed_money_net(), 90_000);
        let r2 = row("Y", "Y", 30_000, 120_000);
        assert_eq!(r2.managed_money_net(), -90_000);
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_managed_money_net_pre_computed() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                row("088691", "GOLD", 120_000, 30_000),
                row("084691", "SILVER", 40_000, 10_000),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &CotConfig::default())
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
        assert_eq!(rows.len(), 2);
        let gold = &rows[0];
        assert_eq!(gold.get("contract_name").unwrap().as_str().unwrap(), "GOLD");
        assert_eq!(
            gold.get("managed_money_net").unwrap().as_i64(),
            Some(90_000)
        );
        let silver = &rows[1];
        assert_eq!(
            silver.get("managed_money_net").unwrap().as_i64(),
            Some(30_000)
        );
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &CotConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &CotConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![row("088691", "GOLD", 120_000, 30_000)],
        };
        let _ = run_cycle(&pool, &fetcher, &CotConfig::default())
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
