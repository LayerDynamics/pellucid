//! Energy domain — six high-priority seeders covering the
//! oil + gas market surface.
//!
//! ## Seeder map
//!
//! | Module                  | Cache key                              | Tier  | Upstream                              |
//! |-------------------------|----------------------------------------|-------|---------------------------------------|
//! | `seed_oil_inventories`  | `eia:petroleum-stocks:latest:v1`       | FAST  | EIA v2 petroleum/stoc/wstk            |
//! | `seed_fuel_prices`      | `energy:fuel-prices:current:v1`        | FAST  | EIA v2 petroleum/pri/gnd              |
//! | `seed_spr_policies`     | `energy:spr-status:current:v1`         | SLOW  | EIA v2 petroleum/sum/snd (SPR product)|
//! | `seed_iea_oil_stocks`   | `energy:iea-oil-stocks:monthly:v1`     | SLOW  | JODI world_oil (IEA proper paywalled) |
//! | `seed_jodi`             | `energy:jodi:latest:v1`                | SLOW  | JODI world_oil demand-side            |
//! | `seed_gie_gas_storage`  | `energy:gie-gas-storage:current:v1`    | FAST  | GIE AGSI per-country                  |

pub mod seed_fuel_prices;
pub mod seed_gie_gas_storage;
pub mod seed_iea_oil_stocks;
pub mod seed_jodi;
pub mod seed_oil_inventories;
pub mod seed_spr_policies;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every energy seeder.
#[derive(Debug, Error)]
pub enum EnergySeederError {
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
