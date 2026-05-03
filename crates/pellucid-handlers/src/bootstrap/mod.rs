//! Bootstrap (cold-start hydration) domain.
//!
//! Routes:
//! - `GET /api/bootstrap/v1/get?tier=fast|slow|both` — see
//!   [`v1::get`].
//!
//! Both tier responses are FAST-tier cached at the gateway layer
//! (60 s `s-maxage`); the webview's `bootstrap.ts` service sets up
//! two `AbortController`s and races them against the OP-4 budgets
//! (3 s fast / 5 s slow).

pub mod keys;
pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the bootstrap `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
