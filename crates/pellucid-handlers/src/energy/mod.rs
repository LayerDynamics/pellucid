//! Energy / commodities domain — M3 family 4.4 panel handlers.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the energy `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
