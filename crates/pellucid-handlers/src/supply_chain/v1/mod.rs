//! `supply_chain/v1/*` route module — T4.5.9 ships handlers here.
//!
//! URL prefix is `/api/supply-chain/v1/...` (kebab-case) even though
//! the Rust module name is `supply_chain` (snake_case).

pub mod summary;

use axum::Router;

use crate::state::AppState;

pub const SUMMARY_PATH: &str = "/api/supply-chain/v1/summary";

/// Build the v1 router.
pub fn router(state: AppState) -> Router {
    Router::new().route(
        SUMMARY_PATH,
        axum::routing::get(summary::handler).with_state(state),
    )
}
