//! `news/v1/*` route module.

pub mod get_breaking;
pub mod list_articles;
pub mod list_live;

use axum::Router;

use crate::state::AppState;

/// Path for the list-articles endpoint. Matches the loader at
/// `webview/src/data/loaders/news/list.ts` (T4.1.1).
pub const LIST_ARTICLES_PATH: &str = "/api/news/v1/list-articles";

/// Path for the live-stream SSE endpoint. Matches the loader at
/// `webview/src/data/loaders/news/live.ts` (T4.1.2).
pub const LIST_LIVE_PATH: &str = "/api/news/v1/list-live";

/// Path for the breaking-news endpoint. Matches the loader at
/// `webview/src/data/loaders/news/breaking.ts` (T4.1.3) which
/// the [`BreakingNewsBanner`] panel polls.
pub const GET_BREAKING_PATH: &str = "/api/news/v1/get-breaking";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            LIST_ARTICLES_PATH,
            axum::routing::get(list_articles::handler).with_state(state.clone()),
        )
        .route(
            LIST_LIVE_PATH,
            axum::routing::get(list_live::handler).with_state(state.clone()),
        )
        .route(
            GET_BREAKING_PATH,
            axum::routing::get(get_breaking::handler).with_state(state),
        )
}
