//! `news/v1/*` route module.

pub mod list_articles;

use axum::Router;

use crate::state::AppState;

/// Path for the list-articles endpoint. Matches the loader at
/// `webview/src/data/loaders/news/list.ts` (T4.1.1).
pub const LIST_ARTICLES_PATH: &str = "/api/news/v1/list-articles";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new().route(
        LIST_ARTICLES_PATH,
        axum::routing::get(list_articles::handler).with_state(state),
    )
}
