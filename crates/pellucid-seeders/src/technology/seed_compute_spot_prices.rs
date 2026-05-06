//! seed_compute_spot_prices — FAST-tier snapshot of EC2
//! on-demand + spot pricing for a basket of common instance
//! types in a single region.
//!
//! Covers the user's "compute pricing" ask. Memory pricing is
//! handled separately in `seed_memory_market` (using publicly-
//! traded memory-vendor stock prices as a proxy for the
//! paywalled DRAM spot market).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::technology::TechnologySeederError;

/// Cache key — NEW FAST tier slot added by T3.8 expansion.
pub const CACHE_KEY: &str = "technology:compute-spot-prices:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "compute-spot-prices-vantage-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "technology-compute";

/// Default region.
pub const DEFAULT_REGION: &str = "us-east-1";

/// Default instance basket — covers GPU (g5/g6/p4d), high-mem
/// (r7i), general (m7i/c7i), and large-memory (x2i) families.
pub const DEFAULT_INSTANCE_TYPES: &[&str] = &[
    "m7i.large",
    "m7i.4xlarge",
    "c7i.large",
    "c7i.4xlarge",
    "r7i.large",
    "r7i.4xlarge",
    "g5.xlarge",
    "g6.xlarge",
    "p4d.24xlarge",
    "x2iedn.xlarge",
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct ComputeSpotPricesConfig {
    /// AWS region.
    pub region: String,
    /// Instance basket.
    pub instance_types: Vec<String>,
}

impl Default for ComputeSpotPricesConfig {
    fn default() -> Self {
        Self {
            region: DEFAULT_REGION.to_string(),
            instance_types: DEFAULT_INSTANCE_TYPES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

/// One per-instance pricing row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PricingRow {
    /// Instance type.
    pub instance_type: String,
    /// Region.
    pub region: String,
    /// Memory (GiB).
    pub memory_gib: f64,
    /// vCPU count.
    pub vcpu: u32,
    /// On-demand $/hr.
    pub ondemand_usd_hr: f64,
    /// Spot minimum $/hr.
    pub spot_min_usd_hr: f64,
    /// Spot maximum $/hr.
    pub spot_max_usd_hr: f64,
    /// Spot average $/hr.
    pub spot_avg_usd_hr: f64,
    /// Pre-computed `spot_avg / ondemand` ratio (0.0 when
    /// either is zero). The "spot discount" indicator the
    /// panel renders.
    pub spot_discount_ratio: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputeSpotPricesSnapshot {
    /// Pricing rows in input-basket order.
    pub rows: Vec<PricingRow>,
    /// Echo of the region.
    pub region: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled pricing — mirrors `pellucid_streams::InstancePricing`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedPricing {
    /// Instance type.
    pub instance_type: String,
    /// Region.
    pub region: String,
    /// Memory (GiB).
    pub memory_gib: f64,
    /// vCPU.
    pub vcpu: u32,
    /// On-demand $/hr.
    pub ondemand_usd_hr: f64,
    /// Spot min $/hr.
    pub spot_min_usd_hr: f64,
    /// Spot max $/hr.
    pub spot_max_usd_hr: f64,
    /// Spot avg $/hr.
    pub spot_avg_usd_hr: f64,
}

/// DI trait — wraps `pellucid_streams::VantageComputeClient::fetch_pricing`.
#[async_trait]
pub trait ComputePricingFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch pricing for the basket in `region`.
    async fn fetch_pricing(
        &self,
        region: &str,
        instance_types: &[&str],
    ) -> Result<Vec<FetchedPricing>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`TechnologySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn ComputePricingFetcher,
    config: &ComputeSpotPricesConfig,
) -> Result<PublishOutcome, TechnologySeederError> {
    let basket: Vec<&str> = config.instance_types.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_pricing(&config.region, &basket)
        .await
        .map_err(|e| TechnologySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(TechnologySeederError::EmptyUpstream);
    }
    let rows: Vec<PricingRow> = fetched
        .into_iter()
        .map(|p| {
            let discount = if p.ondemand_usd_hr == 0.0 || p.spot_avg_usd_hr == 0.0 {
                0.0
            } else {
                p.spot_avg_usd_hr / p.ondemand_usd_hr
            };
            PricingRow {
                instance_type: p.instance_type,
                region: p.region,
                memory_gib: p.memory_gib,
                vcpu: p.vcpu,
                ondemand_usd_hr: p.ondemand_usd_hr,
                spot_min_usd_hr: p.spot_min_usd_hr,
                spot_max_usd_hr: p.spot_max_usd_hr,
                spot_avg_usd_hr: p.spot_avg_usd_hr,
                spot_discount_ratio: discount,
            }
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = ComputeSpotPricesSnapshot {
        rows,
        region: config.region.clone(),
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
    let outcome = atomic_publish(pool, "technology", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedPricing>,
    }

    #[async_trait]
    impl ComputePricingFetcher for StaticFetcher {
        async fn fetch_pricing(
            &self,
            _region: &str,
            _instance_types: &[&str],
        ) -> Result<Vec<FetchedPricing>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    fn pricing(itype: &str, ondemand: f64, spot_avg: f64) -> FetchedPricing {
        FetchedPricing {
            instance_type: itype.into(),
            region: "us-east-1".into(),
            memory_gib: 8.0,
            vcpu: 2,
            ondemand_usd_hr: ondemand,
            spot_min_usd_hr: spot_avg * 0.7,
            spot_max_usd_hr: spot_avg * 1.5,
            spot_avg_usd_hr: spot_avg,
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "technology:compute-spot-prices:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_pre_computes_spot_discount_ratio() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![pricing("m7i.large", 0.10, 0.045)],
        };
        let _ = run_cycle(&pool, &fetcher, &ComputeSpotPricesConfig::default())
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 1);
        // 0.045 / 0.10 = 0.45
        let ratio = rows[0]
            .get("spot_discount_ratio")
            .unwrap()
            .as_f64()
            .unwrap();
        assert!((ratio - 0.45).abs() < 1e-9);
    }

    #[tokio::test]
    async fn run_cycle_zero_division_yields_zero_ratio() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![pricing("p4d.24xlarge", 32.0, 0.0)],
        };
        let _ = run_cycle(&pool, &fetcher, &ComputeSpotPricesConfig::default())
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let ratio = parsed.pointer("/data/rows/0/spot_discount_ratio").unwrap();
        assert!((ratio.as_f64().unwrap() - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &ComputeSpotPricesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, TechnologySeederError::EmptyUpstream));
    }
}
