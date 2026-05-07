//! `correlation/v1/*` — cross-domain convergence endpoints.

pub mod run;

use axum::Router;

use crate::state::AppState;

pub use run::PATH as RUN_PATH;

pub fn router(state: AppState) -> Router {
    Router::new().route(
        RUN_PATH,
        axum::routing::post(run::handler).with_state(state),
    )
}
