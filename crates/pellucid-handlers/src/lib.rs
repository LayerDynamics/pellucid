//! pellucid-handlers — RPC handler implementations.
//!
//! Each domain (`aviation`, `market`, `news`, …) sits in its own
//! module. Generated request / response types live under
//! [`generated`] and are produced from `proto/` by `bun run gen`
//! (`pellucid-codegen`'s build script enforces the proto-tree /
//! generated-tree digest invariant on every cargo build).
//!
//! ## Public surface
//!
//! - [`aviation`] — domain modules; each `vN/get_*` function
//!   returns an [`axum::routing::MethodRouter`] that the gateway
//!   `HandlerSet` mounts.
//! - [`AppState`] — shared per-process state every handler is
//!   parametrised with: SQLite pool, cache coalesce registry, and
//!   pluggable upstream clients.
//! - [`build_handlers`] — composes the per-domain `Router`s into a
//!   single `Router` ready to hand to `pellucid_gateway::build_router`.

pub mod aviation;
pub mod bootstrap;
pub mod climate;
pub mod conflict;
pub mod consumer_prices;
pub mod correlation;
pub mod economic;
pub mod energy;
pub mod forecast;
pub mod generated;
pub mod infra;
pub mod intelligence;
pub mod market;
pub mod military;
pub mod news;
pub mod sanctions;
pub mod state;
pub mod supply_chain;
pub mod telegram;
pub mod thermal;
pub mod trade;

pub use state::{AppState, AppStateError, FlightStatusUpstream};

use axum::Router;

/// Compose every domain's routes into one `Router` ready to hand
/// to `pellucid_gateway::build_router`.
pub fn build_handlers(state: AppState) -> Router {
    Router::new()
        .merge(aviation::router(state.clone()))
        .merge(bootstrap::router(state.clone()))
        .merge(climate::router(state.clone()))
        .merge(conflict::router(state.clone()))
        .merge(consumer_prices::router(state.clone()))
        .merge(correlation::router(state.clone()))
        .merge(economic::router(state.clone()))
        .merge(energy::router(state.clone()))
        .merge(forecast::router(state.clone()))
        .merge(infra::router(state.clone()))
        .merge(intelligence::router(state.clone()))
        .merge(market::router(state.clone()))
        .merge(military::router(state.clone()))
        .merge(news::router(state.clone()))
        .merge(sanctions::router(state.clone()))
        .merge(supply_chain::router(state.clone()))
        .merge(telegram::router(state.clone()))
        .merge(thermal::router(state.clone()))
        .merge(trade::router(state))
}

/// Returns the crate version string from `CARGO_PKG_VERSION`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
        assert!(version().contains('.'));
    }

    #[tokio::test]
    async fn build_handlers_returns_router() {
        let state = AppState::for_tests();
        let _: Router = build_handlers(state);
    }
}
