//! seed_jodi — SLOW-tier snapshot of JODI World Oil monthly
//! demand-side data (TOTDEMO flow breakdown).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::energy::EnergySeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — NEW SLOW tier slot added by T3.8 energy domain.
pub const CACHE_KEY: &str = "energy:jodi:latest:v1";

/// SLOW-tier TTL — 6 hours. JODI publishes monthly with a
/// 3-month reporting lag.
pub const TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "jodi-world-oil-totdemo-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "energy-jodi";

/// Default JODI flow breakdown filter — TOTDEMO = total demand.
pub const DEFAULT_FLOW_BREAKDOWN: &str = "TOTDEMO";

/// Default basket — major oil-consuming countries (ISO-3).
pub const DEFAULT_COUNTRIES: &[&str] = &[
    "USA", "CHN", "IND", "JPN", "DEU", "RUS", "KOR", "FRA", "GBR", "BRA",
];

/// Default JODI dataset to fetch (Oil or Gas).
pub const DEFAULT_DATASET_OIL: bool = true;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct JodiConfig {
    /// Country codes to keep in the snapshot.
    pub countries: Vec<String>,
    /// Flow breakdown filter (e.g. `"TOTDEMO"`).
    pub flow_breakdown: String,
    /// `true` to fetch world_oil; `false` for world_gas.
    pub oil_dataset: bool,
}

impl Default for JodiConfig {
    fn default() -> Self {
        Self {
            countries: DEFAULT_COUNTRIES.iter().map(|s| (*s).to_string()).collect(),
            flow_breakdown: DEFAULT_FLOW_BREAKDOWN.to_string(),
            oil_dataset: DEFAULT_DATASET_OIL,
        }
    }
}

/// One per-country/period reading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DemandRow {
    /// ISO 3-letter country code.
    pub country: String,
    /// `YYYY-MM` reporting period.
    pub time_period: String,
    /// Energy product (`CRUDEOIL`, `MOTOR_GASOLINE`, etc.).
    pub energy_product: String,
    /// Reported value.
    pub obs_value: f64,
    /// Units (`KBD`, `MCM`).
    pub unit_measure: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JodiSnapshot {
    /// Per-country demand rows for the most recent period the
    /// upstream has data for.
    pub rows: Vec<DemandRow>,
    /// Echo of the flow filter.
    pub flow_breakdown: String,
    /// `true` if oil dataset, `false` if gas.
    pub oil_dataset: bool,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled JODI row — mirrors `pellucid_streams::JodiRow`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedJodiRow {
    /// Country.
    pub country: String,
    /// Energy product.
    pub energy_product: String,
    /// Flow breakdown.
    pub flow_breakdown: String,
    /// Units.
    pub unit_measure: String,
    /// Time period (`YYYY-MM`).
    pub time_period: String,
    /// Value.
    pub obs_value: f64,
}

/// DI trait — wraps `pellucid_streams::JodiClient::fetch_world`.
#[async_trait]
pub trait JodiFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch world JODI data for `country` + `flow_breakdown`,
    /// using the oil dataset when `oil_dataset` is true.
    async fn fetch_world(
        &self,
        oil_dataset: bool,
        country: Option<&str>,
        flow_breakdown: Option<&str>,
    ) -> Result<Vec<FetchedJodiRow>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`EnergySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn JodiFetcher,
    config: &JodiConfig,
) -> Result<PublishOutcome, EnergySeederError> {
    let fetched = fetcher
        .fetch_world(config.oil_dataset, None, Some(&config.flow_breakdown))
        .await
        .map_err(|e| EnergySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(EnergySeederError::EmptyUpstream);
    }
    // Pick the most-recent period across the dataset, then keep
    // one row per country in the basket.
    let latest_period = fetched
        .iter()
        .map(|r| r.time_period.clone())
        .max()
        .ok_or(EnergySeederError::EmptyUpstream)?;
    let countries: std::collections::HashSet<String> = config.countries.iter().cloned().collect();
    let mut rows: Vec<DemandRow> = fetched
        .into_iter()
        .filter(|r| r.time_period == latest_period && countries.contains(&r.country))
        .map(|r| DemandRow {
            country: r.country,
            time_period: r.time_period,
            energy_product: r.energy_product,
            obs_value: r.obs_value,
            unit_measure: r.unit_measure,
        })
        .collect();
    if rows.is_empty() {
        return Err(EnergySeederError::EmptyUpstream);
    }
    rows.sort_by(|a, b| {
        b.obs_value
            .partial_cmp(&a.obs_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = JodiSnapshot {
        rows,
        flow_breakdown: config.flow_breakdown.clone(),
        oil_dataset: config.oil_dataset,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(21_600_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "energy", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedJodiRow>,
    }

    #[async_trait]
    impl JodiFetcher for StaticFetcher {
        async fn fetch_world(
            &self,
            _oil_dataset: bool,
            _country: Option<&str>,
            _flow_breakdown: Option<&str>,
        ) -> Result<Vec<FetchedJodiRow>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl JodiFetcher for FailingFetcher {
        async fn fetch_world(
            &self,
            _oil_dataset: bool,
            _country: Option<&str>,
            _flow_breakdown: Option<&str>,
        ) -> Result<Vec<FetchedJodiRow>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn row(country: &str, period: &str, value: f64) -> FetchedJodiRow {
        FetchedJodiRow {
            country: country.into(),
            energy_product: "CRUDEOIL".into(),
            flow_breakdown: "TOTDEMO".into(),
            unit_measure: "KBD".into(),
            time_period: period.into(),
            obs_value: value,
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "energy:jodi:latest:v1");
    }

    #[tokio::test]
    async fn run_cycle_picks_latest_period_filtered_to_basket() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                row("USA", "2026-02", 19_000.0),
                row("USA", "2026-03", 20_000.0),
                row("CHN", "2026-03", 16_500.0),
                row("ZWE", "2026-03", 200.0), // not in basket → dropped
                row("IND", "2026-03", 5_500.0),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &JodiConfig::default())
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
        // All rows belong to the latest period (2026-03).
        assert!(rows
            .iter()
            .all(|r| r.get("time_period").unwrap().as_str().unwrap() == "2026-03"));
        // Sorted by obs_value desc → USA first.
        assert_eq!(rows[0].get("country").unwrap().as_str().unwrap(), "USA");
    }

    #[tokio::test]
    async fn run_cycle_no_basket_match_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![row("ZWE", "2026-03", 200.0)],
        };
        let err = run_cycle(&pool, &fetcher, &JodiConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, EnergySeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &JodiConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, EnergySeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &JodiConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, EnergySeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![row("USA", "2026-03", 20_000.0)],
        };
        let _ = run_cycle(&pool, &fetcher, &JodiConfig::default())
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
