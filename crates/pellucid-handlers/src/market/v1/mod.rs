//! `market/v1/*` route module.

pub mod analyze_stock;
pub mod backtest_stock;
pub mod breadth;
pub mod list_market_quotes;

use axum::Router;

use crate::state::AppState;

/// Path for the list-market-quotes endpoint. Matches the loader at
/// `webview/src/data/loaders/market/list-market-quotes.ts` (T4.2.1).
pub const LIST_MARKET_QUOTES_PATH: &str = "/api/market/v1/list-market-quotes";

/// Path for the analyze-stock endpoint. Matches the loader at
/// `webview/src/data/loaders/market/analyze-stock.ts` (T4.2.2).
/// Tier-2 gated — see [`analyze_stock::REQUIRED_TIER`] for the
/// constant the binary's `RouteEntitlementRules::require()`
/// call uses.
pub const ANALYZE_STOCK_PATH: &str = "/api/market/v1/analyze-stock";

/// Path for the backtest-stock endpoint. Matches the loader at
/// `webview/src/data/loaders/market/backtest-stock.ts` (T4.2.3).
/// Tier-2 gated — see [`backtest_stock::REQUIRED_TIER`].
pub const BACKTEST_STOCK_PATH: &str = "/api/market/v1/backtest-stock";

/// Path for the breadth endpoint. Matches the loader at
/// `webview/src/data/loaders/market/breadth.ts` (T4.2.4).
/// Anonymous tier — breadth is a public surface.
pub const BREADTH_PATH: &str = "/api/market/v1/breadth";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            LIST_MARKET_QUOTES_PATH,
            axum::routing::get(list_market_quotes::handler).with_state(state.clone()),
        )
        .route(
            ANALYZE_STOCK_PATH,
            axum::routing::get(analyze_stock::handler).with_state(state.clone()),
        )
        .route(
            BACKTEST_STOCK_PATH,
            axum::routing::get(backtest_stock::handler).with_state(state.clone()),
        )
        .route(
            BREADTH_PATH,
            axum::routing::get(breadth::handler).with_state(state),
        )
}
