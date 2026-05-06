//! `climate/v1/*` route module — M3 family 4.6.

pub mod air_quality;
pub mod climate_anomalies;
pub mod climate_summary;
pub mod earthquakes;
pub mod natural_events;
pub mod noaa_alerts;
pub mod volcano_activity;
pub mod wildfire;

use axum::Router;

use crate::state::AppState;

pub const SUMMARY_PATH: &str = "/api/climate/v1/summary";
pub const WILDFIRE_PATH: &str = "/api/climate/v1/wildfire";
pub const EARTHQUAKES_PATH: &str = "/api/climate/v1/earthquakes";
pub const AIR_QUALITY_PATH: &str = "/api/climate/v1/air-quality";
pub const VOLCANO_PATH: &str = "/api/climate/v1/volcano-activity";
pub const NOAA_ALERTS_PATH: &str = "/api/climate/v1/noaa-alerts";
pub const ANOMALIES_PATH: &str = "/api/climate/v1/anomalies";
pub const NATURAL_EVENTS_PATH: &str = "/api/climate/v1/natural-events";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            SUMMARY_PATH,
            axum::routing::get(climate_summary::handler).with_state(state.clone()),
        )
        .route(
            WILDFIRE_PATH,
            axum::routing::get(wildfire::handler).with_state(state.clone()),
        )
        .route(
            EARTHQUAKES_PATH,
            axum::routing::get(earthquakes::handler).with_state(state.clone()),
        )
        .route(
            AIR_QUALITY_PATH,
            axum::routing::get(air_quality::handler).with_state(state.clone()),
        )
        .route(
            VOLCANO_PATH,
            axum::routing::get(volcano_activity::handler).with_state(state.clone()),
        )
        .route(
            NOAA_ALERTS_PATH,
            axum::routing::get(noaa_alerts::handler).with_state(state.clone()),
        )
        .route(
            ANOMALIES_PATH,
            axum::routing::get(climate_anomalies::handler).with_state(state.clone()),
        )
        .route(
            NATURAL_EVENTS_PATH,
            axum::routing::get(natural_events::handler).with_state(state),
        )
}
