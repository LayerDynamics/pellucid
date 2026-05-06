//! Trade domain — tariff alerts seeder.

pub mod seed_tariff_alerts;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type.
#[derive(Debug, Error)]
pub enum TradeSeederError {
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
