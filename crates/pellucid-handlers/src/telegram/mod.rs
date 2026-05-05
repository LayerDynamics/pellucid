//! Telegram domain handlers.
//!
//! Routes:
//! - `GET /api/telegram/v1/feed` — see [`v1::feed`].
//!
//! All routes are FAST-tier cached against snapshots written by
//! `pellucid-seeders::intel::seed_telegram_intel_min`.

pub mod v1;

use axum::Router;

use crate::state::AppState;

/// Build the telegram `Router`.
pub fn router(state: AppState) -> Router {
    Router::new().merge(v1::router(state))
}
