//! `sanctions/v1/*` route module — T4.5.8 ships handlers here.

pub mod pressure;

use axum::Router;

use crate::state::AppState;

pub const PRESSURE_PATH: &str = "/api/sanctions/v1/pressure";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new().route(
        PRESSURE_PATH,
        axum::routing::get(pressure::handler).with_state(state),
    )
}
