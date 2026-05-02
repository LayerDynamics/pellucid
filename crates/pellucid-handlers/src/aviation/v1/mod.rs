//! `aviation/v1/*` route module.

pub mod get_flight_status;

use axum::Router;

use crate::state::AppState;

/// Path for the flight-status endpoint. Matches the original
/// WorldMonitor route at
/// `server/aviation/v1/get-flight-status.ts`.
pub const GET_FLIGHT_STATUS_PATH: &str = "/api/aviation/v1/get-flight-status";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new().route(
        GET_FLIGHT_STATUS_PATH,
        axum::routing::get(get_flight_status::handler).with_state(state),
    )
}
