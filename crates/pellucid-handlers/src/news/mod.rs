//! News domain handlers.
//!
//! Routes:
//! - `GET /api/news/v1/list-articles` — see [`v1::list_articles`].
//!
//! All routes are FAST-tier cached. The webview's `NewsPanel`
//! (M3 family 4.1) calls them directly.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the news `Router` containing every news route.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
