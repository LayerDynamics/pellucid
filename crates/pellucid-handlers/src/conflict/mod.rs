//! Conflict / armed-incident domain handlers (T4.5.0 scaffold).
//!
//! Routes land in T4.5.1 (`UcdpEventsPanel`) and beyond. Empty
//! `Router::new()` shipped now so `build_handlers` can wire the
//! family in alphabetical order without re-touching `lib.rs` for
//! every panel task.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the conflict `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
