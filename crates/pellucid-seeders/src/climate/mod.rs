//! Climate domain — five high-priority seeders covering the
//! environmental-monitoring surface area Pellucid panels render.
//!
//! ## Seeder map
//!
//! | Module                  | Cache key                              | Tier  | Upstream                     |
//! |-------------------------|----------------------------------------|-------|------------------------------|
//! | `seed_climate_anomalies`| `climate:latest-anomaly:global:v1`     | FAST  | NOAA NCEI temperature series |
//! | `seed_fire_detections`  | `wildfire:active-perimeters:current:v1`| FAST  | NASA FIRMS VIIRS 24h CSV     |
//! | `seed_earthquakes`      | `seismology:recent-quakes:24h:v1`      | FAST  | USGS Earthquake GeoJSON      |
//! | `seed_natural_events`   | `natural:volcano-feed:current:v1`      | FAST  | NASA EONET v3                |
//! | `seed_air_quality`      | `climate:air-quality:current:v1`       | FAST  | OpenAQ v2 latest (NEW key)   |
//!
//! Each seeder mirrors the `theater_posture` DI-trait pattern.

pub mod seed_air_quality;
pub mod seed_climate_anomalies;
pub mod seed_earthquakes;
pub mod seed_fire_detections;
pub mod seed_natural_events;
pub mod seed_noaa_alerts;
pub mod seed_station_records;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every climate seeder.
#[derive(Debug, Error)]
pub enum ClimateSeederError {
    /// Upstream HTTP client failed.
    #[error("upstream: {0}")]
    Upstream(String),
    /// Atomic-publish failure.
    #[error("publish: {0}")]
    Publish(#[from] PublishError),
    /// Upstream returned no rows — soft error so the scheduler
    /// keeps cycling.
    #[error("upstream returned no data")]
    EmptyUpstream,
}
