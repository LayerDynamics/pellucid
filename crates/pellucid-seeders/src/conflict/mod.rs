//! Conflict domain — five high-priority seeders covering the
//! geopolitical-incident surface area.
//!
//! ## Seeder map
//!
//! | Module                | Cache key                       | Tier  | Upstream                                |
//! |-----------------------|---------------------------------|-------|-----------------------------------------|
//! | `seed_ucdp_events`    | `conflict:events-24h:v1`        | FAST  | UCDP REST GED endpoint                  |
//! | `seed_gdelt_intel`    | `conflict:incident-feed:v1`     | FAST  | GDELT 2.0 DOC API                       |
//! | `seed_iran_events`    | `conflict:iran-events:24h:v1`   | FAST  | GDELT 2.0 DOC API (Iran-filtered)       |
//! | `seed_unrest_events`  | `unrest:events-24h:v1`          | FAST  | GDELT 2.0 DOC API (protest-filtered)    |
//! | `seed_acled`          | `conflict:hot-actors:24h:v1`    | FAST  | ACLED REST API (key + email required)   |

pub mod seed_acled;
pub mod seed_gdelt_intel;
pub mod seed_iran_events;
pub mod seed_thermal_anomalies;
pub mod seed_ucdp_events;
pub mod seed_unrest_events;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every conflict seeder.
#[derive(Debug, Error)]
pub enum ConflictSeederError {
    /// Upstream HTTP client failed.
    #[error("upstream: {0}")]
    Upstream(String),
    /// Atomic-publish failure.
    #[error("publish: {0}")]
    Publish(#[from] PublishError),
    /// Upstream returned no rows.
    #[error("upstream returned no data")]
    EmptyUpstream,
}
