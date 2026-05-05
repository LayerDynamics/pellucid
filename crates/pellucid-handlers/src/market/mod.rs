//! Market / finance domain handlers.
//!
//! Routes:
//! - `GET /api/market/v1/list-market-quotes` — see
//!   [`v1::list_market_quotes`].
//!
//! All routes are FAST-tier cached against snapshots written by
//! the `pellucid-seeders` `markets` module.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the market `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
