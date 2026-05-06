//! `infra/v1/*` route module — M3 family 4.7.

pub mod active_campaigns;
pub mod cloud_status;
pub mod cve_trending;
pub mod cyber_incidents;
pub mod infra_summary;
pub mod internet_outages;

use axum::Router;

use crate::state::AppState;

pub const SUMMARY_PATH: &str = "/api/infra/v1/summary";
pub const INTERNET_OUTAGES_PATH: &str = "/api/infra/v1/internet-outages";
pub const CYBER_INCIDENTS_PATH: &str = "/api/infra/v1/cyber-incidents";
pub const CVE_TRENDING_PATH: &str = "/api/infra/v1/cve-trending";
pub const ACTIVE_CAMPAIGNS_PATH: &str = "/api/infra/v1/active-campaigns";
pub const CLOUD_STATUS_PATH: &str = "/api/infra/v1/cloud-status";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(SUMMARY_PATH, axum::routing::get(infra_summary::handler).with_state(state.clone()))
        .route(INTERNET_OUTAGES_PATH, axum::routing::get(internet_outages::handler).with_state(state.clone()))
        .route(CYBER_INCIDENTS_PATH, axum::routing::get(cyber_incidents::handler).with_state(state.clone()))
        .route(CVE_TRENDING_PATH, axum::routing::get(cve_trending::handler).with_state(state.clone()))
        .route(ACTIVE_CAMPAIGNS_PATH, axum::routing::get(active_campaigns::handler).with_state(state.clone()))
        .route(CLOUD_STATUS_PATH, axum::routing::get(cloud_status::handler).with_state(state))
}
