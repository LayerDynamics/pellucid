//! `correlation/v1/*` route module.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the correlation router. Currently mounts only the
/// `correlation/v1/run` endpoint; future siblings (e.g. cycle
/// status, per-domain queries) land here.
pub fn router(state: AppState) -> Router {
    v1::router(state)
}
