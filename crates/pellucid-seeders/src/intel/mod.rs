//! Intel domain — Telegram intel writes are owned by
//! `pellucid_streams::telegram::run` (T4.5.0). The previous
//! `seed_telegram_intel_min` web-preview scraper was removed at
//! T4.5.0 along with `pellucid_streams::telegram_public`. This
//! module currently has no seeders, but the shared
//! [`IntelSeederError`] is kept because future intel seeders
//! (M5+) will reuse it.

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
