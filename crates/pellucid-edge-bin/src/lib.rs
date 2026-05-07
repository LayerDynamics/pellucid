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
use pellucid_core::vault::EnvVault;
use pellucid_db::{open, DbError, Pool, SqliteOpenOptions};
use pellucid_gateway::{build_router, GatewayConfig};
use pellucid_handlers::{build_handlers, AppState, AppStateError, FlightStatusUpstream};
use pellucid_ml::{build_from_vault, FromVaultError, VaultEngineConfig};
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

/// Build a cloud `MlEngine` from `EnvVault::from_env()`. Returns
/// `Ok(None)` and emits a `tracing::warn!` when keys aren't set so
/// the binary still boots (existing aviation / health paths keep
/// serving). Returns `Ok(Some)` when both `GROQ_API_KEY` and
/// `HF_TOKEN` are set; downstream handlers read `state.ml.is_some()`
/// to decide between 200 (call ML) and 503 (return Retry-After).
async fn build_ml_engine_from_env() -> Option<std::sync::Arc<dyn pellucid_ml::MlEngine>> {
    let vault = EnvVault::from_env();
    match build_from_vault(&vault, &VaultEngineConfig::default()).await {
        Ok(engine) => {
            tracing::info!(
                target: "pellucid::edge::ml",
                "MlEngine constructed from EnvVault — intelligence handlers active"
            );
            Some(engine)
        }
        Err(FromVaultError::MissingKey(name)) => {
            tracing::warn!(
                target: "pellucid::edge::ml",
                missing = name,
                "MlEngine not constructed — intelligence handlers will return 503 until {name} is set"
            );
            None
        }
        Err(other) => {
            tracing::warn!(
                target: "pellucid::edge::ml",
                error = %other,
                "MlEngine construction failed — intelligence handlers will return 503"
            );
            None
        }
    }
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
    let mut state = AppState::new(
        pool.clone(),
        Arc::new(CoalesceRegistry::default()),
        aviation,
    );
    if let Some(ml) = build_ml_engine_from_env().await {
        state = state.with_ml(ml);
    }

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
