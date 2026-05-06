//! Climate / nature domain — M3 family 4.6 panel handlers.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the climate `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
