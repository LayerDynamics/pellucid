//! Markets domain — six high-priority seeders covering the
//! market-data surface area Pellucid panels render.
//!
//! Each seeder follows the same pattern as `theater_posture`:
//! a small DI trait, a `run_cycle` that fetches → assembles a
//! typed snapshot → atomic-publishes via [`crate::atomic_publish`].
//! Production binaries hand in adapters wrapping the
//! `pellucid-streams` HTTP clients; tests hand in adapters with
//! synthetic data.
//!
//! ## Seeder map (all under the `market:*` cache-key namespace)
//!
//! | Module                   | Cache key                              | Tier  | Upstream |
//! |--------------------------|----------------------------------------|-------|----------|
//! | `seed_market_quotes`     | `market:stocks-bootstrap:v1`           | FAST  | Yahoo Finance (v8 chart) |
//! | `seed_commodity_quotes`  | `market:commodities-snapshot:v1`       | FAST  | Yahoo Finance (futures) |
//! | `seed_crypto_quotes`     | `market:crypto-snapshot:v1`            | FAST  | CoinGecko (v3 simple/price) |
//! | `seed_etf_flows`         | `market:etf-flows:current:v1`          | SLOW  | Yahoo Finance (volume × close proxy) |
//! | `seed_gold_etf_flows`    | `market:gold-etf-flows:current:v1`     | SLOW  | Yahoo Finance (gold ETFs) |
//! | `seed_cot`               | `market:cot-report:weekly:v1`          | SLOW  | CFTC Socrata (Disaggregated COT) |
//!
//! All cache keys are written to [`crate::registry::REGISTRY`]
//! one-for-one — the scheduler runs each seeder on its
//! SPEC-001 §17.7 cadence (5 min / 15 min / weekly).

pub mod seed_commodity_quotes;
pub mod seed_cot;
pub mod seed_crypto_quotes;
pub mod seed_etf_flows;
pub mod seed_gold_etf_flows;
pub mod seed_market_quotes;

use thiserror::Error;

use crate::atomic_publish::PublishError;

/// Shared error type for every market seeder. Each seeder
/// re-exports it so callers don't depend on per-module errors.
#[derive(Debug, Error)]
pub enum MarketsSeederError {
    /// Upstream HTTP client failed.
    #[error("upstream: {0}")]
    Upstream(String),
    /// Atomic-publish failure.
    #[error("publish: {0}")]
    Publish(#[from] PublishError),
    /// Upstream returned an empty result set — the seeder treats
    /// this as a soft failure (no data to publish) rather than a
    /// hard error so the scheduler keeps cycling.
    #[error("upstream returned no data")]
    EmptyUpstream,
}
