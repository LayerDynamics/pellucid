//! `intelligence/v1/*` route module.

pub mod classify_event;
pub mod country_brief;
pub mod country_deep_dive;
pub mod extract_entities;
pub mod gdelt_feed;
pub mod regional;
pub mod summarize_article;

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

/// Tier-2 ML endpoints — paths are mirrored in
/// `pellucid-auth::ML_ENDPOINT_ENTITLEMENTS`.
pub use classify_event::PATH as CLASSIFY_EVENT_PATH;
pub use extract_entities::PATH as EXTRACT_ENTITIES_PATH;
pub use summarize_article::PATH as SUMMARIZE_ARTICLE_PATH;

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
            axum::routing::post(extract_entities::handler).with_state(state.clone()),
        )
        .route(
            SUMMARIZE_ARTICLE_PATH,
            axum::routing::post(summarize_article::handler).with_state(state.clone()),
        )
        .route(
            CLASSIFY_EVENT_PATH,
            axum::routing::post(classify_event::handler).with_state(state),
        )
}
