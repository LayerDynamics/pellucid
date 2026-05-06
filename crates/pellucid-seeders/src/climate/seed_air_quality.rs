//! seed_air_quality — FAST-tier snapshot of the latest air-
//! quality readings from OpenAQ.
//!
//! The webview's `AirQualityPanel` renders one badge per
//! city/station for each tracked country. The seeder fetches
//! the latest measurement per station via OpenAQ's
//! `v2/latest` endpoint, capped at `limit` rows so the panel
//! doesn't have to handle the full 50k-station dump.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::climate::ClimateSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — NEW FAST tier slot added by T3.8 climate domain.
pub const CACHE_KEY: &str = "climate:air-quality:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "air-quality-openaq-v2-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "air-quality";

/// Default countries — US + the seven other markets where
/// the panel publishes city-level breakdowns.
pub const DEFAULT_COUNTRIES: &[&str] = &["US", "CA", "MX", "GB", "DE", "FR", "JP", "IN"];

/// Default station-row cap.
pub const DEFAULT_LIMIT: u32 = 100;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct AirQualityConfig {
    /// ISO-3166 alpha-2 country codes.
    pub countries: Vec<String>,
    /// Maximum stations to fetch per cycle.
    pub limit: u32,
}

impl Default for AirQualityConfig {
    fn default() -> Self {
        Self {
            countries: DEFAULT_COUNTRIES.iter().map(|s| (*s).to_string()).collect(),
            limit: DEFAULT_LIMIT,
        }
    }
}

/// One station row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AirQualityStationRow {
    /// Station name.
    pub location: String,
    /// City.
    pub city: String,
    /// Country code.
    pub country: String,
    /// Latitude.
    pub latitude: f64,
    /// Longitude.
    pub longitude: f64,
    /// Per-parameter latest measurements.
    pub measurements: Vec<AirMeasurementRow>,
}

/// One measurement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AirMeasurementRow {
    /// Pollutant code (`pm25`, `no2`, etc.).
    pub parameter: String,
    /// Value.
    pub value: f64,
    /// Unit string.
    pub unit: String,
    /// ISO-8601 timestamp.
    pub last_updated: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AirQualitySnapshot {
    /// Stations the upstream returned, in upstream order.
    pub rows: Vec<AirQualityStationRow>,
    /// Echo of the country list the seeder asked for.
    pub countries: Vec<String>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled station — mirrors `pellucid_streams::AirQualityStation`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedStation {
    /// Station name.
    pub location: String,
    /// City.
    pub city: String,
    /// Country.
    pub country: String,
    /// Latitude.
    pub latitude: f64,
    /// Longitude.
    pub longitude: f64,
    /// Measurements.
    pub measurements: Vec<FetchedMeasurement>,
}

/// Distilled measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedMeasurement {
    /// Pollutant code.
    pub parameter: String,
    /// Value.
    pub value: f64,
    /// Unit.
    pub unit: String,
    /// ISO-8601 timestamp.
    pub last_updated: String,
}

/// DI trait — wraps `pellucid_streams::OpenAqClient::fetch_latest`.
#[async_trait]
pub trait AirQualityFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest measurements for the country list.
    async fn fetch_latest(
        &self,
        countries: &[&str],
        limit: u32,
    ) -> Result<Vec<FetchedStation>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`ClimateSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn AirQualityFetcher,
    config: &AirQualityConfig,
) -> Result<PublishOutcome, ClimateSeederError> {
    let country_refs: Vec<&str> = config.countries.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_latest(&country_refs, config.limit)
        .await
        .map_err(|e| ClimateSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(ClimateSeederError::EmptyUpstream);
    }

    let rows: Vec<AirQualityStationRow> = fetched
        .into_iter()
        .map(|s| AirQualityStationRow {
            location: s.location,
            city: s.city,
            country: s.country,
            latitude: s.latitude,
            longitude: s.longitude,
            measurements: s
                .measurements
                .into_iter()
                .map(|m| AirMeasurementRow {
                    parameter: m.parameter,
                    value: m.value,
                    unit: m.unit,
                    last_updated: m.last_updated,
                })
                .collect(),
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = AirQualitySnapshot {
        rows,
        countries: config.countries.clone(),
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
    let outcome = atomic_publish(pool, "climate", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        stations: Vec<FetchedStation>,
    }

    #[async_trait]
    impl AirQualityFetcher for StaticFetcher {
        async fn fetch_latest(
            &self,
            _countries: &[&str],
            _limit: u32,
        ) -> Result<Vec<FetchedStation>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.stations.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl AirQualityFetcher for FailingFetcher {
        async fn fetch_latest(
            &self,
            _countries: &[&str],
            _limit: u32,
        ) -> Result<Vec<FetchedStation>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn station(location: &str, city: &str, country: &str, pm25: f64) -> FetchedStation {
        FetchedStation {
            location: location.into(),
            city: city.into(),
            country: country.into(),
            latitude: 34.05,
            longitude: -118.24,
            measurements: vec![FetchedMeasurement {
                parameter: "pm25".into(),
                value: pm25,
                unit: "µg/m³".into(),
                last_updated: "2026-05-04T16:00:00+00:00".into(),
            }],
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "climate:air-quality:current:v1");
    }

    #[test]
    fn default_basket_includes_eight_countries() {
        let cfg = AirQualityConfig::default();
        assert_eq!(cfg.countries.len(), 8);
        assert_eq!(cfg.limit, DEFAULT_LIMIT);
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_two_stations() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            stations: vec![
                station("Downtown LA", "Los Angeles", "US", 18.4),
                station("Brooklyn — IS 314", "New York", "US", 22.1),
            ],
        };
        let outcome = run_cycle(&pool, &fetcher, &AirQualityConfig::default())
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
        assert_eq!(
            rows[0].get("location").unwrap().as_str().unwrap(),
            "Downtown LA"
        );
        let m0 = rows[0].get("measurements").unwrap().as_array().unwrap();
        assert_eq!(m0.len(), 1);
        assert_eq!(m0[0].get("parameter").unwrap().as_str().unwrap(), "pm25");
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { stations: vec![] };
        let err = run_cycle(&pool, &fetcher, &AirQualityConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ClimateSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &AirQualityConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ClimateSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            stations: vec![station("LA", "LA", "US", 18.4)],
        };
        let _ = run_cycle(&pool, &fetcher, &AirQualityConfig::default())
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

    #[tokio::test]
    async fn run_cycle_echoes_countries_in_snapshot() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            stations: vec![station("X", "X", "DE", 5.0)],
        };
        let cfg = AirQualityConfig {
            countries: vec!["DE".into(), "FR".into()],
            limit: 50,
        };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let countries = parsed
            .pointer("/data/countries")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(countries.len(), 2);
    }
}
