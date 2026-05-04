//! seed_climate_anomalies — FAST-tier snapshot of the latest
//! NOAA NCEI global land+ocean temperature anomaly readings.
//!
//! The webview's `ClimatePanel` renders the most-recent annual
//! anomaly + a 10-year sparkline. The seeder fetches the full
//! series (NCEI streams it as one ~25 KiB JSON), keeps the
//! tail (latest 10 years) for the sparkline, and pins the
//! single most-recent reading as the headline figure.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::climate::ClimateSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in
/// `pellucid_handlers::bootstrap::keys::FAST_KEYS`.
pub const CACHE_KEY: &str = "climate:latest-anomaly:global:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "climate-anomalies-noaa-ncei-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "climate-anomalies";

/// Default sparkline window — 10 years.
pub const DEFAULT_SPARKLINE_YEARS: usize = 10;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct ClimateAnomaliesConfig {
    /// Calendar year passed to the upstream as the
    /// `1880-{end}` range. Production uses the current year;
    /// tests pin a fixed value.
    pub end_year: u16,
    /// How many years of history to keep in the sparkline tail.
    pub sparkline_years: usize,
}

impl Default for ClimateAnomaliesConfig {
    fn default() -> Self {
        Self {
            end_year: today_year(),
            sparkline_years: DEFAULT_SPARKLINE_YEARS,
        }
    }
}

/// One annual anomaly reading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnomalyReading {
    /// Calendar year.
    pub year: u16,
    /// °C anomaly relative to 20th-century mean.
    pub anomaly_c: f64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnomaliesSnapshot {
    /// Series title from the upstream description.
    pub title: String,
    /// Units (always `"degrees Celsius"`).
    pub units: String,
    /// Most-recent annual reading (the headline figure).
    pub latest: AnomalyReading,
    /// Last `sparkline_years` readings, oldest-first.
    pub sparkline: Vec<AnomalyReading>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled series — mirrors `pellucid_streams::TemperatureAnomalySeries`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedSeries {
    /// Series title.
    pub title: String,
    /// Units.
    pub units: String,
    /// Annual readings, oldest-first.
    pub readings: Vec<AnomalyReading>,
}

/// DI trait — wraps `pellucid_streams::NoaaNceiClient::fetch_global_land_ocean`.
#[async_trait]
pub trait ClimateAnomaliesFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the global land+ocean anomaly series ending at
    /// `end_year`.
    async fn fetch_global_land_ocean(
        &self,
        end_year: u16,
    ) -> Result<FetchedSeries, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`ClimateSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn ClimateAnomaliesFetcher,
    config: &ClimateAnomaliesConfig,
) -> Result<PublishOutcome, ClimateSeederError> {
    let series = fetcher
        .fetch_global_land_ocean(config.end_year)
        .await
        .map_err(|e| ClimateSeederError::Upstream(e.to_string()))?;
    if series.readings.is_empty() {
        return Err(ClimateSeederError::EmptyUpstream);
    }
    let latest = series
        .readings
        .last()
        .cloned()
        .ok_or(ClimateSeederError::EmptyUpstream)?;
    let cutoff = series.readings.len().saturating_sub(config.sparkline_years);
    let sparkline = series.readings[cutoff..].to_vec();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = AnomaliesSnapshot {
        title: series.title,
        units: series.units,
        latest,
        sparkline,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(60_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.sparkline.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "climate", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn today_year() -> u16 {
    let secs = pellucid_core::now_ms() / 1000;
    let days = secs / 86_400 + 719_468;
    let era = if days >= 0 { days / 146_097 } else { (days - 146_096) / 146_097 };
    let doe = (days - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + i64::from(m <= 2)) as u16
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        series: FetchedSeries,
    }

    #[async_trait]
    impl ClimateAnomaliesFetcher for StaticFetcher {
        async fn fetch_global_land_ocean(
            &self,
            _end_year: u16,
        ) -> Result<FetchedSeries, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.series.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl ClimateAnomaliesFetcher for FailingFetcher {
        async fn fetch_global_land_ocean(
            &self,
            _end_year: u16,
        ) -> Result<FetchedSeries, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn series(years: u16, anomalies: &[f64]) -> FetchedSeries {
        FetchedSeries {
            title: "Global Land and Ocean".into(),
            units: "degrees Celsius".into(),
            readings: anomalies
                .iter()
                .enumerate()
                .map(|(i, a)| AnomalyReading {
                    year: years + i as u16,
                    anomaly_c: *a,
                })
                .collect(),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "climate:latest-anomaly:global:v1");
    }

    #[test]
    fn config_default_yields_at_least_2026() {
        let cfg = ClimateAnomaliesConfig::default();
        assert!(cfg.end_year >= 2026);
        assert_eq!(cfg.sparkline_years, DEFAULT_SPARKLINE_YEARS);
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_sparkline_tail() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            series: series(
                2010,
                &[0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.05, 1.08, 1.10, 1.12, 1.15, 1.18, 1.20],
            ),
        };
        let outcome = run_cycle(
            &pool,
            &fetcher,
            &ClimateAnomaliesConfig {
                end_year: 2022,
                sparkline_years: 5,
            },
        )
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
        let latest = parsed.pointer("/data/latest").unwrap();
        assert_eq!(latest.get("year").unwrap().as_u64(), Some(2022));
        assert!((latest.get("anomaly_c").unwrap().as_f64().unwrap() - 1.20).abs() < 1e-9);
        let sparkline = parsed.pointer("/data/sparkline").unwrap().as_array().unwrap();
        assert_eq!(sparkline.len(), 5);
        assert_eq!(sparkline[0].get("year").unwrap().as_u64(), Some(2018));
    }

    #[tokio::test]
    async fn run_cycle_short_series_returns_full_sparkline() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            series: series(2024, &[1.10, 1.15, 1.20]),
        };
        let _ = run_cycle(
            &pool,
            &fetcher,
            &ClimateAnomaliesConfig {
                end_year: 2026,
                sparkline_years: 10,
            },
        )
        .await
        .unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let sparkline = parsed.pointer("/data/sparkline").unwrap().as_array().unwrap();
        assert_eq!(sparkline.len(), 3);
    }

    #[tokio::test]
    async fn run_cycle_empty_series_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            series: FetchedSeries {
                title: "x".into(),
                units: "x".into(),
                readings: vec![],
            },
        };
        let err = run_cycle(&pool, &fetcher, &ClimateAnomaliesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ClimateSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &ClimateAnomaliesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ClimateSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            series: series(2024, &[1.10, 1.20]),
        };
        let _ = run_cycle(&pool, &fetcher, &ClimateAnomaliesConfig::default())
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
