//! Supply-chain / corridor-risk domain handlers (T4.5.0 scaffold).
//!
//! Routes land in T4.5.9 (`SupplyChainPanel`). The Rust module name
//! is `supply_chain` (snake_case); the URL prefix is
//! `/api/supply-chain/v1/...` (kebab-case) to match the URL conventions
//! the other families use. Empty `Router::new()` shipped now.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the supply-chain `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
