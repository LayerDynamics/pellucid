//! `military/v1/*` route module — T4.5.2 / 4.5.3 / 4.5.4 / 4.5.7
//! ship handlers here.

pub mod defense_patents;
pub mod military_correlation;
pub mod strategic_posture;
pub mod strategic_risk;

use axum::Router;

use crate::state::AppState;

pub const STRATEGIC_POSTURE_PATH: &str = "/api/military/v1/strategic-posture";
pub const STRATEGIC_RISK_PATH: &str = "/api/military/v1/strategic-risk";
pub const MILITARY_CORRELATION_PATH: &str = "/api/military/v1/military-correlation";
pub const DEFENSE_PATENTS_PATH: &str = "/api/military/v1/defense-patents";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            STRATEGIC_POSTURE_PATH,
            axum::routing::get(strategic_posture::handler).with_state(state.clone()),
        )
        .route(
            STRATEGIC_RISK_PATH,
            axum::routing::get(strategic_risk::handler).with_state(state.clone()),
        )
        .route(
            MILITARY_CORRELATION_PATH,
            axum::routing::get(military_correlation::handler).with_state(state.clone()),
        )
        .route(
            DEFENSE_PATENTS_PATH,
            axum::routing::get(defense_patents::handler).with_state(state),
        )
}
