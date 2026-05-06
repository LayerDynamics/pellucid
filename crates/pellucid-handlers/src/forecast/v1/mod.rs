//! `forecast/v1/*` route module — M3 family 4.8.

pub mod extended;
pub mod now_cast;
pub mod prediction_markets;
pub mod scenario_library;
pub mod scenario_state;
pub mod summary;

use axum::Router;

use crate::state::AppState;

pub const NOW_CAST_PATH: &str = "/api/forecast/v1/now-cast";
pub const PREDICTION_MARKETS_PATH: &str = "/api/forecast/v1/prediction-markets";
pub const SCENARIO_STATE_PATH: &str = "/api/forecast/v1/scenario-state";
pub const SCENARIO_LIBRARY_PATH: &str = "/api/forecast/v1/scenario-library";
pub const EXTENDED_PATH: &str = "/api/forecast/v1/extended";
pub const SUMMARY_PATH: &str = "/api/forecast/v1/summary";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(NOW_CAST_PATH, axum::routing::get(now_cast::handler).with_state(state.clone()))
        .route(PREDICTION_MARKETS_PATH, axum::routing::get(prediction_markets::handler).with_state(state.clone()))
        .route(SCENARIO_STATE_PATH, axum::routing::get(scenario_state::handler).with_state(state.clone()))
        .route(SCENARIO_LIBRARY_PATH, axum::routing::get(scenario_library::handler).with_state(state.clone()))
        .route(EXTENDED_PATH, axum::routing::get(extended::handler).with_state(state.clone()))
        .route(SUMMARY_PATH, axum::routing::get(summary::handler).with_state(state))
}
