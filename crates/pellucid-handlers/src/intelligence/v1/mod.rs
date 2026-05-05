//! `intelligence/v1/*` route module.

pub mod country_deep_dive;
pub mod gdelt_feed;
pub mod regional;

use axum::Router;

use crate::state::AppState;

/// Path for the GDELT feed endpoint. Matches the loader at
/// `webview/src/data/loaders/intel/gdelt.ts` (T4.1.4).
pub const GDELT_FEED_PATH: &str = "/api/intelligence/v1/gdelt-feed";

/// Path for the regional rollup endpoint. Matches the loader at
/// `webview/src/data/loaders/intel/regional.ts` (T4.1.6).
pub const REGIONAL_PATH: &str = "/api/intelligence/v1/regional";

/// Path for the country deep-dive endpoint. Matches the loader at
/// `webview/src/data/loaders/intel/country-deep-dive.ts` (T4.1.7).
pub const COUNTRY_DEEP_DIVE_PATH: &str = "/api/intelligence/v1/country-deep-dive";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            GDELT_FEED_PATH,
            axum::routing::get(gdelt_feed::handler).with_state(state.clone()),
        )
        .route(
            REGIONAL_PATH,
            axum::routing::get(regional::handler).with_state(state.clone()),
        )
        .route(
            COUNTRY_DEEP_DIVE_PATH,
            axum::routing::get(country_deep_dive::handler).with_state(state),
        )
}
