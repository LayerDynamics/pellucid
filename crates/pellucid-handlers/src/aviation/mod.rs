//! Aviation domain handlers.
//!
//! Routes:
//! - `GET /api/aviation/v1/get-flight-status` — see [`v1::get_flight_status`].
//!
//! All routes are FAST-tier cached (60 s `s-maxage`). The webview's
//! `aviation` panel calls them directly.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the aviation `Router` containing every aviation route.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
