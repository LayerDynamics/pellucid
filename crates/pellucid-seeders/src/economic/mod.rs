//! Economic / consumer-prices domain seeders for M3 family 4.3.
//!
//! Each seeder follows the same DI-trait pattern as the markets
//! domain — production binaries wire concrete pellucid-streams
//! adapters at boot, tests inject deterministic fakes.

pub mod seed_big_mac_index;
pub mod seed_fao_food_price;
pub mod seed_financial_stress;
pub mod seed_grocery_basket;
pub mod seed_gulf_economies;
pub mod seed_national_debt;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type — same shape as `MarketsSeederError` so the
/// scheduler treats them uniformly.
#[derive(Debug, Error)]
pub enum EconomicSeederError {
    /// Upstream HTTP client failed.
    #[error("upstream: {0}")]
    Upstream(String),
    /// Atomic-publish failure.
    #[error("publish: {0}")]
    Publish(#[from] PublishError),
    /// Upstream returned an empty result set.
    #[error("upstream returned no data")]
    EmptyUpstream,
}
