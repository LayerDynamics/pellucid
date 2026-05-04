//! Logistics domain — supply-chain stress and port congestion.
//!
//! Both seeders publish derived signals: stress-index from
//! GDELT theme coverage, port-congestion from the AIS state
//! accumulator's per-port-bbox vessel counts. Paid sources
//! (FBX, Drewry WCI, Project44) hold the "real" supply-chain
//! and port-congestion datasets.

pub mod seed_port_congestion;
pub mod seed_supply_chain_stress;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type.
#[derive(Debug, Error)]
pub enum LogisticsSeederError {
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
