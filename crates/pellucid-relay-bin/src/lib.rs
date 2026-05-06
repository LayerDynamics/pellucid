//! pellucid-relay-bin — library surface.
//!
//! The relay's behaviour is exposed as a thin library so the
//! integration tests can drive the assembly end-to-end without
//! spawning a child process.
//!
//! Boot pipeline (T3.10 / SPEC-001 §17.5):
//!
//! 1. **C1 startup gate** — [`ensure_safe_to_boot`] validates
//!    the env-var tuple before touching any I/O.
//! 2. **DB open + migrations** — `pellucid_db::open` with the
//!    configured `db_url`.
//! 3. **Background tasks** — AIS WebSocket → `MaritimeState`
//!    accumulator (when `AIS_API_KEY` is set). Seeder
//!    scheduler runs every registered seeder on its
//!    SPEC-001 §17.7 cadence.
//! 4. **Axum server** — `:3004` mounting `/health` (real
//!    cascade — **L3 fix**), `/metrics`, `/opensky/*` proxy.
//! 5. **Shutdown** — `Ctrl-C` (SIGINT) drains background
//!    tasks within `shutdown_grace` then exits zero.

pub mod ais_task;
pub mod app;
pub mod auth;
pub mod config;
pub mod health;
pub mod metrics;
pub mod proxy;
pub mod startup_check;
pub mod telegram_task;

pub use app::{build_app, BootedRelay, RelayBootError};
pub use auth::{require_shared_secret, SharedSecret, RELAY_SECRET_HEADER};
pub use config::{Config, ConfigError, ConfigSource};
pub use health::{
    compute_cascade, health_router, status_code_for, GroupState, GroupStatus, HealthBody,
    HealthState, HealthStatus, HealthSummary, HEALTH_PATH,
};
pub use metrics::{install_recorder, metrics_router, MetricsError, MetricsState, METRICS_PATH};
pub use proxy::{opensky_router, ProxyState, OPENSKY_PROXY_PREFIX};
pub use startup_check::{ensure_safe_to_boot, BootDecision, StartupEnv, StartupError};

/// Returns the crate version string from `CARGO_PKG_VERSION`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        let v = version();
        assert!(!v.is_empty());
        assert!(v.contains('.'));
    }
}
