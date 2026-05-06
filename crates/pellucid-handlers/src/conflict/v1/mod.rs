//! `conflict/v1/*` route module — T4.5.1 + T4.5.5 ship handlers here.

pub mod escalation_correlation;
pub mod ucdp_events;

use axum::Router;

use crate::state::AppState;

pub const UCDP_EVENTS_PATH: &str = "/api/conflict/v1/ucdp-events";
pub const ESCALATION_CORRELATION_PATH: &str = "/api/conflict/v1/escalation-correlation";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            UCDP_EVENTS_PATH,
            axum::routing::get(ucdp_events::handler).with_state(state.clone()),
        )
        .route(
            ESCALATION_CORRELATION_PATH,
            axum::routing::get(escalation_correlation::handler).with_state(state),
        )
}
