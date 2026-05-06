//! `economic/v1/*` route module.

pub mod big_mac;
pub mod fao_food_price_index;
pub mod financial_stress_index;
pub mod fuel_prices;
pub mod gulf_economies;
pub mod macro_signals;
pub mod macro_tiles;
pub mod national_debt;
pub mod shared;
pub mod snapshot;

use axum::Router;

use crate::state::AppState;

pub const SNAPSHOT_PATH: &str = "/api/economic/v1/snapshot";
pub const FSI_PATH: &str = "/api/economic/v1/financial-stress-index";
pub const MACRO_SIGNALS_PATH: &str = "/api/economic/v1/macro-signals";
pub const MACRO_TILES_PATH: &str = "/api/economic/v1/macro-tiles";
pub const NATIONAL_DEBT_PATH: &str = "/api/economic/v1/national-debt";
pub const BIG_MAC_PATH: &str = "/api/economic/v1/big-mac";
pub const FUEL_PRICES_PATH: &str = "/api/economic/v1/fuel-prices";
pub const FAO_FOOD_PRICE_INDEX_PATH: &str = "/api/economic/v1/fao-food-price-index";
pub const GULF_ECONOMIES_PATH: &str = "/api/economic/v1/gulf-economies";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            SNAPSHOT_PATH,
            axum::routing::get(snapshot::handler).with_state(state.clone()),
        )
        .route(
            FSI_PATH,
            axum::routing::get(financial_stress_index::handler).with_state(state.clone()),
        )
        .route(
            MACRO_SIGNALS_PATH,
            axum::routing::get(macro_signals::handler).with_state(state.clone()),
        )
        .route(
            MACRO_TILES_PATH,
            axum::routing::get(macro_tiles::handler).with_state(state.clone()),
        )
        .route(
            NATIONAL_DEBT_PATH,
            axum::routing::get(national_debt::handler).with_state(state.clone()),
        )
        .route(
            BIG_MAC_PATH,
            axum::routing::get(big_mac::handler).with_state(state.clone()),
        )
        .route(
            FUEL_PRICES_PATH,
            axum::routing::get(fuel_prices::handler).with_state(state.clone()),
        )
        .route(
            FAO_FOOD_PRICE_INDEX_PATH,
            axum::routing::get(fao_food_price_index::handler).with_state(state.clone()),
        )
        .route(
            GULF_ECONOMIES_PATH,
            axum::routing::get(gulf_economies::handler).with_state(state),
        )
}
