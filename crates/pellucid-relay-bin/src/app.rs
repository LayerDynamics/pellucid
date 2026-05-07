//! Relay app builder — opens the DB, installs the metrics
//! recorder, mounts every route, and returns the booted
//! relay so the binary's `main` (or an integration test) can
//! drive the Axum server + shutdown coordination.
//!
//! The split between `build_app` (this file) and `main.rs`
//! mirrors `pellucid-edge-bin` — every byte of behaviour is
//! reachable from a library function so the integration test
//! never spawns a child process.

use std::sync::Arc;

use axum::middleware;
use axum::Router;
use thiserror::Error;
use tokio::task::JoinHandle;

use pellucid_db::{open, DbError, Pool, SqliteOpenOptions};
use pellucid_streams::{MaritimeState, OpenSkyClient, OpenSkyConfig};

use crate::ais_task::{self, AisHandles};
use crate::auth::{require_shared_secret, SharedSecret};
use crate::config::Config;
use crate::health::{HealthState, DEFAULT_OUTAGE_THRESHOLD};
use crate::metrics::{install_recorder, MetricsError, MetricsState};
use crate::proxy::ProxyState;
use crate::scenario_task::{self, ScenarioHandle};

/// Errors `build_app` can surface.
#[derive(Debug, Error)]
pub enum RelayBootError {
    /// SQLite open / migration failure.
    #[error("db: {0}")]
    Db(#[from] DbError),
    /// Metrics recorder install failure.
    #[error("metrics: {0}")]
    Metrics(#[from] MetricsError),
    /// `reqwest::Client` build failure (rare).
    #[error("http client: {0}")]
    HttpClient(String),
}

/// The fully-wired relay. The caller binds the listener,
/// runs the Axum server, and on shutdown calls
/// [`BootedRelay::shutdown`] to drain background tasks.
pub struct BootedRelay {
    /// Mounted Axum router.
    pub app: Router,
    /// SQLite pool — exposed so the seeder scheduler (when
    /// the caller chooses to spawn it) can reuse the same
    /// pool instead of opening a second connection set.
    pub pool: Pool,
    /// Shared maritime state (the AIS task feeds it; maritime
    /// + logistics seeders read snapshots).
    pub maritime_state: MaritimeState,
    /// Tokio handles for the AIS pipeline. `None` when the
    /// relay was booted in dev mode without `AIS_API_KEY`.
    pub ais_handles: Option<AisHandles>,
    /// Resolved config (echoed for telemetry / tests).
    pub config: Config,
    /// Optional scheduler join handle. The integration test
    /// passes `None` for `seeder_jobs`; production wires the
    /// full set.
    pub scheduler: Option<JoinHandle<()>>,
    /// Optional scenario worker handle. `None` when `UPSTASH_REDIS_REST_*`
    /// env vars are absent (dev mode).
    pub scenario_handle: Option<ScenarioHandle>,
    /// Optional Telegram MTProto run-task handle (T4.5.0). `None`
    /// when the relay was booted without `TELEGRAM_API_ID` /
    /// `TELEGRAM_API_HASH` (dev mode). Field present only with
    /// the `telegram` feature; the dep links `grammers → libsql`
    /// which collides with `sqlx → libsqlite3-sys` at link time
    /// until the session backend is moved off libsql.
    #[cfg(feature = "telegram")]
    pub telegram_handles: Option<crate::telegram_task::TelegramTaskHandle>,
}

impl std::fmt::Debug for BootedRelay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootedRelay")
            .field("listen_addr", &self.config.listen_addr)
            .field("db_url", &self.config.db_url)
            .field("dev_mode", &self.config.relay_shared_secret.is_none())
            .field("ais_running", &self.ais_handles.is_some())
            .field("scheduler_running", &self.scheduler.is_some())
            .finish_non_exhaustive()
    }
}

impl BootedRelay {
    /// Drain background tasks. Returns `true` iff every task
    /// exited cleanly within `config.shutdown_grace`.
    pub async fn shutdown(self) -> bool {
        let mut clean = true;
        if let Some(handles) = self.ais_handles {
            if !ais_task::shutdown(handles, self.config.shutdown_grace).await {
                clean = false;
            }
        }
        if let Some(handle) = self.scenario_handle {
            if !scenario_task::shutdown(handle, self.config.shutdown_grace).await {
                clean = false;
            }
        }
        #[cfg(feature = "telegram")]
        if let Some(handle) = self.telegram_handles {
            if !crate::telegram_task::shutdown(handle, self.config.shutdown_grace).await {
                clean = false;
            }
        }
        if let Some(scheduler) = self.scheduler {
            scheduler.abort();
            if tokio::time::timeout(self.config.shutdown_grace, scheduler)
                .await
                .is_err()
            {
                clean = false;
            }
        }
        clean
    }
}

/// Build the relay app. Tests pass `seeder_jobs = vec![]` to
/// avoid spawning live cycles. Production wires the full
/// `pellucid_seeders` registry via the production seeder
/// factory.
///
/// `expected_groups` is the cascade-group list the
/// `/health` endpoint compares against. Production passes
/// the union of every registered seeder's `cascade_group`.
///
/// # Errors
/// See [`RelayBootError`].
pub async fn build_app(
    config: Config,
    seeder_jobs: Vec<pellucid_seeders::scheduler::ScheduledJob>,
    expected_groups: Vec<String>,
) -> Result<BootedRelay, RelayBootError> {
    let pool = open(SqliteOpenOptions {
        url: config.db_url.clone(),
        max_connections: 8,
        run_migrations: true,
    })
    .await?;

    let metrics_handle = install_recorder()?;

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| RelayBootError::HttpClient(e.to_string()))?;
    // OpenSky's OAuth2 client requires non-empty credentials —
    // dev-mode boot supplies empty strings, which causes the
    // upstream call to fail at fetch time (NOT at boot time).
    // The proxy's error handler surfaces the failure cleanly.
    let opensky = Arc::new(OpenSkyClient::new(
        OpenSkyConfig::production(
            config.opensky_client_id.clone().unwrap_or_default(),
            config.opensky_client_secret.clone().unwrap_or_default(),
        ),
        http,
    ));
    let proxy_state = ProxyState {
        opensky: opensky.clone(),
    };

    let secret = SharedSecret::new(config.relay_shared_secret.clone());
    let proxy_router = crate::proxy::opensky_router(proxy_state).layer(
        middleware::from_fn_with_state(secret, require_shared_secret),
    );

    let health_state = HealthState {
        pool: pool.clone(),
        outage_threshold: DEFAULT_OUTAGE_THRESHOLD,
        expected_groups,
    };
    let health_router = crate::health::health_router(health_state);

    let metrics_router = crate::metrics::metrics_router(MetricsState {
        handle: metrics_handle,
    });

    let app = Router::new()
        .merge(health_router)
        .merge(metrics_router)
        .merge(proxy_router);

    let maritime_state = MaritimeState::new();
    let ais_handles = ais_task::spawn(config.ais_api_key.clone(), maritime_state.clone());

    // Scenario worker — spawns iff UPSTASH_REDIS_REST_URL/TOKEN are set in
    // the process env. Dev mode boots without it.
    let scenario_handle = scenario_task::spawn();

    #[cfg(feature = "telegram")]
    let telegram_handles = match crate::telegram_task::try_spawn(pool.clone(), &config).await {
        Ok(handles) => handles,
        Err(err) => {
            tracing::warn!(
                target: "pellucid::relay::telegram",
                error = %err,
                "telegram run task failed to start; relay continues without it"
            );
            None
        }
    };

    let scheduler = if seeder_jobs.is_empty() {
        None
    } else {
        let stats = pellucid_seeders::scheduler::SchedulerStats::new();
        let tick_limit = config.scheduler_tick_limit;
        Some(tokio::spawn(async move {
            pellucid_seeders::scheduler::run_scheduler(seeder_jobs, stats, tick_limit).await;
        }))
    };

    Ok(BootedRelay {
        app,
        pool,
        maritime_state,
        ais_handles,
        config,
        scheduler,
        scenario_handle,
        #[cfg(feature = "telegram")]
        telegram_handles,
    })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::config::ConfigSource;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn dev_config() -> Config {
        Config::parse(&ConfigSource::default()).unwrap()
    }

    #[tokio::test]
    async fn build_app_default_dev_config_succeeds() {
        let cfg = dev_config();
        let booted = build_app(cfg, vec![], vec![]).await.unwrap();
        assert!(booted.ais_handles.is_none());
        assert!(booted.scheduler.is_none());
        let _ = booted.shutdown().await;
    }

    #[tokio::test]
    async fn health_route_responds_in_dev_mode() {
        let booted = build_app(dev_config(), vec![], vec![]).await.unwrap();
        let resp = booted
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Empty cascade → Ok (the missing-fraction guard treats
        // total = max(1) so 0/1 = 0 < threshold).
        assert_eq!(resp.status(), StatusCode::OK);
        let _ = booted.shutdown().await;
    }

    #[tokio::test]
    async fn metrics_route_returns_text() {
        let booted = build_app(dev_config(), vec![], vec![]).await.unwrap();
        let resp = booted
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let _ = booted.shutdown().await;
    }

    #[tokio::test]
    async fn opensky_proxy_requires_secret_when_configured() {
        let mut cfg = dev_config();
        cfg.relay_shared_secret = Some("s3cret".into());
        let booted = build_app(cfg, vec![], vec![]).await.unwrap();
        let resp = booted
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/opensky/states/all")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let _ = booted.shutdown().await;
    }

    #[tokio::test]
    async fn opensky_proxy_dev_mode_passes_through_auth() {
        // Dev mode skips the secret check, so the request must
        // reach the proxy handler. We hit a path the handler
        // rejects internally (path doesn't start with
        // `/states/`) — that yields a deterministic 400 without
        // depending on the live OpenSky upstream.
        let booted = build_app(dev_config(), vec![], vec![]).await.unwrap();
        let resp = booted
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/opensky/states/path/invalid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // 400 proves middleware passed through and the handler
        // ran; 401 would indicate the auth layer rejected.
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let _ = booted.shutdown().await;
    }
}
