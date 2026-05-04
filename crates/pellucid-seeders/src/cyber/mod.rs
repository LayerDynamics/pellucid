//! Cyber domain — seeders covering NVD CVE feeds.
//!
//! `seed_cyber_incident_feed` and `seed_cve_trending` fill the
//! two FAST-tier cyber slots that the existing CISA KEV
//! `seed_security_advisories` (in `infra/`) does not cover:
//! `cyber:incident-feed:24h:v1` and `cyber:cve-trending:v1`.

pub mod seed_cve_trending;
pub mod seed_cyber_incident_feed;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every cyber seeder.
#[derive(Debug, Error)]
pub enum CyberSeederError {
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
