//! Prediction domain — two seeders covering Polymarket
//! prediction markets and Metaculus crowd forecasts.

pub mod seed_forecasts;
pub mod seed_polymarket;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every prediction seeder.
#[derive(Debug, Error)]
pub enum PredictionSeederError {
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
