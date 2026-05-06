//! Infra / cyber domain — M3 family 4.7 panel handlers.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the infra `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
