//! Intelligence domain handlers.
//!
//! Routes:
//! - `GET /api/intelligence/v1/gdelt-feed` — see [`v1::gdelt_feed`].
//!
//! All routes are FAST-tier cached against snapshots written by
//! the `pellucid-seeders` `conflict` / `intel` modules.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the intelligence `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
