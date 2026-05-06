//! `consumer-prices/v1/*` route module.

pub mod grocery_basket;
pub mod list;

use axum::Router;

use crate::state::AppState;

pub const LIST_PATH: &str = "/api/consumer-prices/v1/list";
pub const GROCERY_BASKET_PATH: &str = "/api/consumer-prices/v1/grocery-basket";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            LIST_PATH,
            axum::routing::get(list::handler).with_state(state.clone()),
        )
        .route(
            GROCERY_BASKET_PATH,
            axum::routing::get(grocery_basket::handler).with_state(state),
        )
}
