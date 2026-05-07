//! `intelligence/v1/*` route module.

pub mod country_brief;
pub mod country_deep_dive;
pub mod extract_entities;
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

/// Path for the country brief endpoint. Matches the loader at
/// `webview/src/data/loaders/intel/country-brief.ts` (T4.1.8).
pub const COUNTRY_BRIEF_PATH: &str = "/api/intelligence/v1/country-brief";

/// Path for the ML-backed entity-extraction endpoint. Tier-2
/// (`api_starter`); see `crates/pellucid-auth/src/endpoint_tiers.rs`.
pub use extract_entities::PATH as EXTRACT_ENTITIES_PATH;

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
            axum::routing::get(country_deep_dive::handler).with_state(state.clone()),
        )
        .route(
            COUNTRY_BRIEF_PATH,
            axum::routing::get(country_brief::handler).with_state(state.clone()),
        )
        .route(
            EXTRACT_ENTITIES_PATH,
            axum::routing::post(extract_entities::handler).with_state(state),
        )
}
