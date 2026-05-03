//! pellucid-edge-bin — library surface.
//!
//! The binary's behaviour is exposed as a thin library so the
//! `tests/boot.rs` integration test can drive the assembly
//! end-to-end (open db → build state → mount router) without
//! spawning a child process.

pub mod config;
pub mod middleware;

use std::sync::Arc;

use axum::Router;
use thiserror::Error;

use pellucid_cache::CoalesceRegistry;
use pellucid_db::{open, DbError, Pool, SqliteOpenOptions};
use pellucid_gateway::{build_router, GatewayConfig};
use pellucid_handlers::{build_handlers, AppState, AppStateError, FlightStatusUpstream};
use pellucid_streams::aviationstack::{AviationstackClient, AviationstackConfig};

pub use config::{Config, ConfigError, ConfigSource};
pub use middleware::{healthcheck_router, HealthcheckBody, HEALTHCHECK_PATH};

/// Errors construction may surface.
#[derive(Debug, Error)]
pub enum EdgeBootError {
    /// Config parse failure.
    #[error("config: {0}")]
    Config(#[from] ConfigError),
    /// Sqlite open / migration failure.
    #[error("db: {0}")]
    Db(#[from] DbError),
    /// AppState boot failure (sqlx + migration wrapped).
    #[error("app state: {0}")]
    AppState(#[from] AppStateError),
}

/// Adapter wrapping the production `pellucid-streams`
/// `AviationstackClient` so it satisfies
/// [`FlightStatusUpstream`] without `pellucid-handlers`
/// depending on `pellucid-streams` directly (which would cycle).
#[derive(Debug)]
struct StreamsAviationAdapter(AviationstackClient);

#[async_trait::async_trait]
impl FlightStatusUpstream for StreamsAviationAdapter {
    async fn fetch_flight(
        &self,
        flight: &str,
        date: &str,
        origin: &str,
    ) -> Result<
        Option<pellucid_handlers::generated::aviation::v1::FlightStatus>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        match self.0.fetch_flight(flight, date, origin).await {
            Ok(opt) => Ok(opt),
            Err(e) => Err(Box::new(e)),
        }
    }
}

/// Build the full edge router from a [`Config`]: opens SQLite,
/// runs migrations, wires the aviationstack client, mounts
/// `/healthz` + the gateway-wrapped handlers.
///
/// Returns `(router, pool)` so the caller can carry the pool
/// into background tasks (seeders, cache eviction).
///
/// # Errors
/// See [`EdgeBootError`].
pub async fn build_app(cfg: &Config) -> Result<(Router, Pool), EdgeBootError> {
    let pool = open(SqliteOpenOptions {
        url: cfg.db_url.clone(),
        max_connections: 8,
        run_migrations: true,
    })
    .await?;

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let aviation = Arc::new(StreamsAviationAdapter(AviationstackClient::new(
        AviationstackConfig {
            base_url: cfg.aviationstack_base_url.clone(),
            access_key: cfg.aviationstack_api_key.clone(),
        },
        http,
    )));
    let state = AppState::new(
        pool.clone(),
        Arc::new(CoalesceRegistry::default()),
        aviation,
    );

    let handlers = build_handlers(state);
    let gateway = build_router(handlers, GatewayConfig::permissive_for_tests());

    let app = Router::new().merge(gateway).merge(healthcheck_router());

    Ok((app, pool))
}

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
        assert!(!version().is_empty());
        assert!(version().contains('.'));
    }

    #[tokio::test]
    async fn build_app_with_default_config_succeeds() {
        // Default config uses sqlite::memory: → fully self-
        // contained smoke test of the assembly.
        let cfg = Config::parse(&ConfigSource::default()).unwrap();
        let (router, _pool) = build_app(&cfg).await.unwrap();
        let _ = router; // type-check only
    }
}
