//! `market/v1/*` route module.

pub mod analyze_stock;
pub mod backtest_stock;
pub mod breadth;
pub mod cot;
pub mod daily_brief;
pub mod earnings;
pub mod etf_flows;
pub mod fear_greed;
pub mod liquidity_shifts;
pub mod list_market_quotes;
pub mod stablecoins;
pub mod yield_curve;

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

/// Path for the etf-flows endpoint. Matches the loader at
/// `webview/src/data/loaders/market/etf-flows.ts` (T4.2.5).
/// Anonymous tier — public surface.
pub const ETF_FLOWS_PATH: &str = "/api/market/v1/etf-flows";

/// Path for the fear-greed composite endpoint. Matches the
/// loader at `webview/src/data/loaders/market/fear-greed.ts`
/// (T4.2.6). Anonymous tier — composite is built from public
/// FAST cache slots only.
pub const FEAR_GREED_PATH: &str = "/api/market/v1/fear-greed";

/// Path for the COT positioning endpoint. Matches the loader at
/// `webview/src/data/loaders/market/cot.ts` (T4.2.7). Anonymous
/// tier — CFTC public data.
pub const COT_PATH: &str = "/api/market/v1/cot";

/// Path for the earnings calendar endpoint. Matches the loader
/// at `webview/src/data/loaders/market/earnings.ts` (T4.2.8).
/// Anonymous tier.
pub const EARNINGS_PATH: &str = "/api/market/v1/earnings";

/// Path for the yield curve endpoint. Matches the loader at
/// `webview/src/data/loaders/market/yield-curve.ts` (T4.2.9).
/// Anonymous tier.
pub const YIELD_CURVE_PATH: &str = "/api/market/v1/yield-curve";

/// Path for the stablecoin snapshot endpoint. Matches the loader
/// at `webview/src/data/loaders/market/stablecoins.ts` (T4.2.10).
/// Anonymous tier.
pub const STABLECOINS_PATH: &str = "/api/market/v1/stablecoins";

/// Path for the liquidity-shifts endpoint. Matches the loader at
/// `webview/src/data/loaders/market/liquidity-shifts.ts`
/// (T4.2.11). Anonymous tier.
pub const LIQUIDITY_SHIFTS_PATH: &str = "/api/market/v1/liquidity-shifts";

/// Path for the daily-brief composer endpoint. Matches the
/// loader at `webview/src/data/loaders/market/daily-brief.ts`
/// (T4.2.12). Anonymous tier.
pub const DAILY_BRIEF_PATH: &str = "/api/market/v1/daily-brief";

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
            axum::routing::get(breadth::handler).with_state(state.clone()),
        )
        .route(
            ETF_FLOWS_PATH,
            axum::routing::get(etf_flows::handler).with_state(state.clone()),
        )
        .route(
            FEAR_GREED_PATH,
            axum::routing::get(fear_greed::handler).with_state(state.clone()),
        )
        .route(
            COT_PATH,
            axum::routing::get(cot::handler).with_state(state.clone()),
        )
        .route(
            EARNINGS_PATH,
            axum::routing::get(earnings::handler).with_state(state.clone()),
        )
        .route(
            YIELD_CURVE_PATH,
            axum::routing::get(yield_curve::handler).with_state(state.clone()),
        )
        .route(
            STABLECOINS_PATH,
            axum::routing::get(stablecoins::handler).with_state(state.clone()),
        )
        .route(
            LIQUIDITY_SHIFTS_PATH,
            axum::routing::get(liquidity_shifts::handler).with_state(state.clone()),
        )
        .route(
            DAILY_BRIEF_PATH,
            axum::routing::get(daily_brief::handler).with_state(state),
        )
}
