//! Technology domain — seeders covering software trending,
//! AI/ML model activity, cloud-provider status, and compute
//! pricing.
//!
//! ## Seeder map
//!
//! | Module                        | Cache key                                   | Tier  | Upstream                  |
//! |-------------------------------|---------------------------------------------|-------|---------------------------|
//! | `seed_hackernews_top`         | `technology:hn-top-stories:v1`              | FAST  | HN Firebase API           |
//! | `seed_github_trending`        | `technology:github-trending:v1`             | FAST  | OSSInsight v1/trends/repos|
//! | `seed_huggingface_trending`   | `technology:ai-models-trending:v1`          | FAST  | huggingface.co/api/models |
//! | `seed_cloud_status`           | `technology:cloud-status:current:v1`        | FAST  | GCP status.json (multi-cloud) |
//! | `seed_compute_spot_prices`    | `technology:compute-spot-prices:current:v1` | FAST  | Vantage instances.json    |
//! | `seed_memory_market`          | `technology:memory-market-index:v1`         | FAST  | Yahoo Finance (MU/STX/WDC proxy)|
//!
//! ## Memory pricing — honest constraint
//!
//! There is no free public API for current DRAM / NAND spot
//! pricing — DRAMeXchange, TrendForce, and IDC all paywall
//! their data. The closest free signal Pellucid can publish is
//! the share-price index of major memory manufacturers
//! (Micron, Western Digital, Seagate). `seed_memory_market`
//! computes that index from the Yahoo Finance client and
//! documents the proxy choice in the snapshot's `source_note`.

pub mod seed_cloud_status;
pub mod seed_compute_spot_prices;
pub mod seed_github_trending;
pub mod seed_hackernews_top;
pub mod seed_huggingface_trending;
pub mod seed_memory_market;
pub mod seed_semiconductor_ppi;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every technology seeder.
#[derive(Debug, Error)]
pub enum TechnologySeederError {
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
