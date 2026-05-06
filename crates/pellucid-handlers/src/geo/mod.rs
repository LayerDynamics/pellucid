//! Geo / military domain — M3 family 4.5 panel handlers.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the geo `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
