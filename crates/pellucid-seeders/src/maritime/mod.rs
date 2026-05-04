//! Maritime domain — three seeders covering global vessel
//! density, chokepoint snapshots, and incident reporting.
//!
//! ## Seeder map
//!
//! | Module                  | Cache key                              | Tier  | Upstream                                    |
//! |-------------------------|----------------------------------------|-------|---------------------------------------------|
//! | `seed_ais_snapshot`     | `maritime:ais-snapshot:summary:v1`     | FAST  | MaritimeState (AIS WebSocket accumulator)   |
//! | `seed_chokepoint_status`| `maritime:chokepoint-status:current:v1`| FAST  | MaritimeState (per-bbox vessel counts)      |
//! | `seed_maritime_incidents` | `maritime:active-incidents:v1`       | FAST  | GDELT (maritime + KILL/ATTACK theme query)  |

pub mod seed_ais_snapshot;
pub mod seed_chokepoint_status;
pub mod seed_maritime_incidents;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every maritime seeder.
#[derive(Debug, Error)]
pub enum MaritimeSeederError {
    /// Upstream HTTP / state read failed.
    #[error("upstream: {0}")]
    Upstream(String),
    /// Atomic-publish failure.
    #[error("publish: {0}")]
    Publish(#[from] PublishError),
    /// Upstream returned no rows.
    #[error("upstream returned no data")]
    EmptyUpstream,
}
