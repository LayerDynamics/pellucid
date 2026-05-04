//! Aviation domain — three high-priority seeders covering the
//! airspace surface area Pellucid panels render.
//!
//! ## Seeder map
//!
//! | Module                | Cache key                            | Tier  | Upstream                        |
//! |-----------------------|--------------------------------------|-------|---------------------------------|
//! | `seed_aviation_status`| `aviation:breaking-incidents:v1`     | FAST  | aviationstack (flight watchlist)|
//! | `seed_notam`          | `aviation:active-notams:v1`          | FAST  | FAA NOTAM Search (`notams.aim.faa.gov`) |
//! | `seed_gpsjam`         | `aviation:airspace-restrictions:v1`  | FAST  | gpsjam.org (daily H3 hexagons)  |
//!
//! Each seeder mirrors the `theater_posture` DI-trait pattern.
//! Production binaries hand in adapters wrapping the
//! `pellucid-streams` HTTP clients; tests inject static
//! fetchers.

pub mod seed_aviation_status;
pub mod seed_gpsjam;
pub mod seed_notam;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every aviation seeder.
#[derive(Debug, Error)]
pub enum AviationSeederError {
    /// Upstream HTTP client failed.
    #[error("upstream: {0}")]
    Upstream(String),
    /// Atomic-publish failure.
    #[error("publish: {0}")]
    Publish(#[from] PublishError),
    /// Upstream returned no rows — soft error so the scheduler
    /// keeps cycling without raising an alert.
    #[error("upstream returned no data")]
    EmptyUpstream,
}
