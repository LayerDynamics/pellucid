//! seed_tariff_alerts — FAST-tier tariff-policy alert snapshot.
//! Production adapters wire to USTR press releases + WTO TBT
//! notifications + EU CBAM bulletins.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::trade::TradeSeederError;

/// Cache key — FAST tier.
pub const CACHE_KEY: &str = "trade:tariff-alerts:current:v1";

/// 30 m TTL.
pub const TTL: Duration = Duration::from_secs(30 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "tariff-alerts-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "trade";

/// One tariff-alert row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TariffRow {
    /// Source authority — `USTR`, `WTO`, `EU_DG_TRADE`.
    pub authority: String,
    /// Origin country ISO.
    pub origin: String,
    /// Destination country ISO.
    pub destination: String,
    /// Affected HS-code subset (e.g. `8542`).
    pub hs_code: String,
    /// Product class human label.
    pub product: String,
    /// Tariff rate change in percentage points.
    pub rate_delta_pp: f64,
    /// ISO-8601 effective stamp.
    pub effective: String,
    /// Brief description.
    pub headline: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TariffSnapshot {
    /// Rows sorted descending by effective date.
    pub rows: Vec<TariffRow>,
    /// Sum of `rate_delta_pp` across rows (signed).
    pub total_rate_delta_pp: f64,
    /// Total row count.
    pub total: usize,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedTariffRow {
    /// Authority.
    pub authority: String,
    /// Origin ISO.
    pub origin: String,
    /// Destination ISO.
    pub destination: String,
    /// HS code.
    pub hs_code: String,
    /// Product label.
    pub product: String,
    /// Rate delta.
    pub rate_delta_pp: f64,
    /// Effective.
    pub effective: String,
    /// Headline.
    pub headline: String,
}

/// DI trait.
#[async_trait]
pub trait TariffAlertsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch tariff-policy alerts.
    async fn fetch_alerts(
        &self,
    ) -> Result<Vec<FetchedTariffRow>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn TariffAlertsFetcher,
) -> Result<PublishOutcome, TradeSeederError> {
    let fetched = fetcher
        .fetch_alerts()
        .await
        .map_err(|e| TradeSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(TradeSeederError::EmptyUpstream);
    }
    let total_rate_delta_pp: f64 = fetched.iter().map(|r| r.rate_delta_pp).sum();
    let mut rows: Vec<TariffRow> = fetched
        .into_iter()
        .map(|r| TariffRow {
            authority: r.authority,
            origin: r.origin,
            destination: r.destination,
            hs_code: r.hs_code,
            product: r.product,
            rate_delta_pp: r.rate_delta_pp,
            effective: r.effective,
            headline: r.headline,
        })
        .collect();
    rows.sort_by(|a, b| b.effective.cmp(&a.effective));
    let total = rows.len();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = TariffSnapshot {
        rows,
        total_rate_delta_pp,
        total,
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
    let outcome = atomic_publish(pool, "trade", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedTariffRow>,
    }

    #[async_trait]
    impl TariffAlertsFetcher for StaticFetcher {
        async fn fetch_alerts(
            &self,
        ) -> Result<Vec<FetchedTariffRow>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    fn t(authority: &str, eff: &str, delta: f64) -> FetchedTariffRow {
        FetchedTariffRow {
            authority: authority.into(),
            origin: "CN".into(),
            destination: "US".into(),
            hs_code: "8542".into(),
            product: "Semiconductors".into(),
            rate_delta_pp: delta,
            effective: eff.into(),
            headline: "Section 301 increase".into(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "trade:tariff-alerts:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_sums_and_sorts() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                t("USTR", "2026-04-28", 25.0),
                t("WTO", "2026-04-30", -5.0),
                t("EU_DG_TRADE", "2026-04-29", 10.0),
            ],
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let total = parsed
            .pointer("/data/total_rate_delta_pp")
            .unwrap()
            .as_f64()
            .unwrap();
        assert!((total - 30.0).abs() < 0.001);
        let auths: Vec<&str> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("authority").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(auths, vec!["WTO", "EU_DG_TRADE", "USTR"]);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, TradeSeederError::EmptyUpstream));
    }
}
