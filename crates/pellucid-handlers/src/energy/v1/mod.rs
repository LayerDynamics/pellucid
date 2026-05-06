//! `energy/v1/*` route module — M3 family 4.4.

pub mod complex;
pub mod crisis;
pub mod gold_intelligence;
pub mod hormuz;
pub mod oil_inventories;
pub mod renewable;

use axum::Router;

use crate::state::AppState;

pub const COMPLEX_PATH: &str = "/api/energy/v1/complex";
pub const CRISIS_PATH: &str = "/api/energy/v1/crisis";
pub const OIL_INVENTORIES_PATH: &str = "/api/energy/v1/oil-inventories";
pub const HORMUZ_PATH: &str = "/api/energy/v1/hormuz";
pub const RENEWABLE_PATH: &str = "/api/energy/v1/renewable";
pub const GOLD_INTELLIGENCE_PATH: &str = "/api/commodities/v1/gold-intelligence";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            COMPLEX_PATH,
            axum::routing::get(complex::handler).with_state(state.clone()),
        )
        .route(
            CRISIS_PATH,
            axum::routing::get(crisis::handler).with_state(state.clone()),
        )
        .route(
            OIL_INVENTORIES_PATH,
            axum::routing::get(oil_inventories::handler).with_state(state.clone()),
        )
        .route(
            HORMUZ_PATH,
            axum::routing::get(hormuz::handler).with_state(state.clone()),
        )
        .route(
            RENEWABLE_PATH,
            axum::routing::get(renewable::handler).with_state(state.clone()),
        )
        .route(
            GOLD_INTELLIGENCE_PATH,
            axum::routing::get(gold_intelligence::handler).with_state(state),
        )
}
