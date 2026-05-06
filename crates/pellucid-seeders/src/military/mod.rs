//! Military domain — active-deployments seeder. Theater
//! posture lives in `crate::theater_posture` (T3.7).
//!
//! Real-time military deployment data is paywalled (Janes,
//! Jane's IHS, USNI Fleet Tracker requires subscription); the
//! free signal is news coverage of deployments via GDELT.

pub mod seed_active_deployments;
pub mod seed_defense_patents;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type.
#[derive(Debug, Error)]
pub enum MilitarySeederError {
    /// Upstream HTTP failure.
    #[error("upstream: {0}")]
    Upstream(String),
    /// Atomic-publish failure.
    #[error("publish: {0}")]
    Publish(#[from] PublishError),
    /// Upstream returned no rows.
    #[error("upstream returned no data")]
    EmptyUpstream,
}
