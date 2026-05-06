//! `trade/v1/*` route module — T4.5.10 ships handlers here.

pub mod policy;

use axum::Router;

use crate::state::AppState;

pub const POLICY_PATH: &str = "/api/trade/v1/policy";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new().route(
        POLICY_PATH,
        axum::routing::get(policy::handler).with_state(state),
    )
}
