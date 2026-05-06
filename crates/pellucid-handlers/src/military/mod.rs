//! Military / strategic-posture domain handlers (T4.5.0 scaffold).
//!
//! Routes land in T4.5.2 (`StrategicPosturePanel`) onwards. Empty
//! `Router::new()` shipped now so the Geo family scaffold mirrors the
//! Markets precedent set by commit `8c5bb7b`.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the military `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
