//! Consumer-prices domain handlers — M3 family 4.3.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the consumer-prices `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
