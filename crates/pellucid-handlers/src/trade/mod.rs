//! Trade / policy domain handlers (T4.5.0 scaffold).
//!
//! Routes land in T4.5.10 (`TradePolicyPanel`). Empty `Router::new()`
//! shipped now to keep `build_handlers` stable across the panel tasks.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the trade `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
