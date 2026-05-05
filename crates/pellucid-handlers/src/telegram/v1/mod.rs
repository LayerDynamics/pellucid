//! `telegram/v1/*` route module.

pub mod feed;

use axum::Router;

use crate::state::AppState;

/// Path for the Telegram feed endpoint. Matches the loader at
/// `webview/src/data/loaders/intel/telegram.ts` (T4.1.5).
pub const FEED_PATH: &str = "/api/telegram/v1/feed";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new().route(
        FEED_PATH,
        axum::routing::get(feed::handler).with_state(state),
    )
}
