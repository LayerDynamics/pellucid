//! `thermal/v1/*` route module — T4.5.6 ships handlers here.

pub mod escalation;

use axum::Router;

use crate::state::AppState;

pub const ESCALATION_PATH: &str = "/api/thermal/v1/escalation";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new().route(
        ESCALATION_PATH,
        axum::routing::get(escalation::handler).with_state(state),
    )
}
