//! `market/v1/*` route module.

pub mod list_market_quotes;

use axum::Router;

use crate::state::AppState;

/// Path for the list-market-quotes endpoint. Matches the loader at
/// `webview/src/data/loaders/market/list-market-quotes.ts` (T4.2.1).
pub const LIST_MARKET_QUOTES_PATH: &str = "/api/market/v1/list-market-quotes";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new().route(
        LIST_MARKET_QUOTES_PATH,
        axum::routing::get(list_market_quotes::handler).with_state(state),
    )
}
