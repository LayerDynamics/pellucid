//! Infrastructure domain — two seeders covering internet
//! outages and security advisories.

pub mod seed_internet_outages;
pub mod seed_security_advisories;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every infra seeder.
#[derive(Debug, Error)]
pub enum InfraSeederError {
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
