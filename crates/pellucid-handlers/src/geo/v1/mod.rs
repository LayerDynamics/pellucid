//! `geo/v1/*` route module — M3 family 4.5.

pub mod defense_patents;
pub mod escalation_correlation;
pub mod military_correlation;
pub mod sanctions_pressure;
pub mod strategic_posture;
pub mod strategic_risk;
pub mod supply_chain;
pub mod thermal_escalation;
pub mod trade_policy;
pub mod ucdp_events;

use axum::Router;

use crate::state::AppState;

pub const UCDP_EVENTS_PATH: &str = "/api/geo/v1/ucdp-events";
pub const STRATEGIC_POSTURE_PATH: &str = "/api/geo/v1/strategic-posture";
pub const STRATEGIC_RISK_PATH: &str = "/api/geo/v1/strategic-risk";
pub const MILITARY_CORRELATION_PATH: &str = "/api/geo/v1/military-correlation";
pub const ESCALATION_CORRELATION_PATH: &str = "/api/geo/v1/escalation-correlation";
pub const THERMAL_ESCALATION_PATH: &str = "/api/geo/v1/thermal-escalation";
pub const DEFENSE_PATENTS_PATH: &str = "/api/geo/v1/defense-patents";
pub const SANCTIONS_PRESSURE_PATH: &str = "/api/geo/v1/sanctions-pressure";
pub const SUPPLY_CHAIN_PATH: &str = "/api/geo/v1/supply-chain";
pub const TRADE_POLICY_PATH: &str = "/api/geo/v1/trade-policy";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(UCDP_EVENTS_PATH, axum::routing::get(ucdp_events::handler).with_state(state.clone()))
        .route(STRATEGIC_POSTURE_PATH, axum::routing::get(strategic_posture::handler).with_state(state.clone()))
        .route(STRATEGIC_RISK_PATH, axum::routing::get(strategic_risk::handler).with_state(state.clone()))
        .route(MILITARY_CORRELATION_PATH, axum::routing::get(military_correlation::handler).with_state(state.clone()))
        .route(ESCALATION_CORRELATION_PATH, axum::routing::get(escalation_correlation::handler).with_state(state.clone()))
        .route(THERMAL_ESCALATION_PATH, axum::routing::get(thermal_escalation::handler).with_state(state.clone()))
        .route(DEFENSE_PATENTS_PATH, axum::routing::get(defense_patents::handler).with_state(state.clone()))
        .route(SANCTIONS_PRESSURE_PATH, axum::routing::get(sanctions_pressure::handler).with_state(state.clone()))
        .route(SUPPLY_CHAIN_PATH, axum::routing::get(supply_chain::handler).with_state(state.clone()))
        .route(TRADE_POLICY_PATH, axum::routing::get(trade_policy::handler).with_state(state))
}
