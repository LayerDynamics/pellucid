//! Intel domain — minimal Telegram public-channel scrape
//! per the SPEC-001 §17.7 "limited" note (full Telegram in
//! M3 week 14).

pub mod seed_telegram_intel_min;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every intel seeder.
#[derive(Debug, Error)]
pub enum IntelSeederError {
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
