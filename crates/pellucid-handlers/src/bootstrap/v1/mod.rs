//! `bootstrap/v1/*` route module.

pub mod get;

use axum::Router;

use crate::state::AppState;

/// Path for the bootstrap-hydration endpoint. Matches the original
/// WorldMonitor route at `api/bootstrap.js`.
pub const BOOTSTRAP_PATH: &str = "/api/bootstrap/v1/get";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new().route(
        BOOTSTRAP_PATH,
        axum::routing::get(get::handler).with_state(state),
    )
}
