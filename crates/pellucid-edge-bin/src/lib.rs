//! pellucid-edge-bin — library surface.
//!
//! The binary's behaviour is exposed as a thin library so the
//! `tests/boot.rs` integration test can drive the assembly
//! end-to-end (open db → build state → mount router) without
//! spawning a child process.

pub mod config;
pub mod middleware;

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use thiserror::Error;
use tower::{Service, ServiceExt};

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

    let mut app = Router::new().merge(gateway).merge(healthcheck_router());

    // SPA fallback — when `cfg.webview_dist` is set (resolved from
    // `PELLUCID_WEBVIEW_DIST` env or the `/srv/webview` filesystem
    // default), mount `tower-http::ServeDir` as the fallback service
    // so any request that didn't match a gateway route or `/healthz`
    // serves the SPA bundle. `not_found_service` falls back to
    // `index.html` so client-side routing works (every route the
    // React Router knows resolves to the SPA shell).
    //
    // SaaS host split: when `cfg.api_host_prefix` is set
    // (`Some("api.")` per SPEC-001 §17), the fallback is wrapped in
    // `HostAwareSpaFallback` so requests with `Host:
    // api.<anything>` get a 404 for non-API paths instead of the
    // SPA. The gateway routes still match under any hostname; only
    // the catch-all fallback is gated. This keeps the spec's "same
    // binary, separate route map" property without standing up a
    // second deployment.
    if let Some(dir) = cfg.webview_dist.as_deref() {
        let index_html = std::path::Path::new(dir).join("index.html");
        let serve = tower_http::services::ServeDir::new(dir)
            .not_found_service(tower_http::services::ServeFile::new(index_html));
        if let Some(prefix) = cfg.api_host_prefix.as_deref() {
            let host_aware = HostAwareSpaFallback {
                inner: serve,
                api_host_prefix: prefix.to_string(),
            };
            app = app.fallback_service(host_aware);
            tracing::info!(
                target: "pellucid::edge::spa",
                path = dir,
                api_host_prefix = prefix,
                "serving SPA bundle from {dir} (404 on Host:{prefix}*)"
            );
        } else {
            app = app.fallback_service(serve);
            tracing::info!(
                target: "pellucid::edge::spa",
                path = dir,
                "serving SPA bundle from {dir}"
            );
        }
    } else {
        tracing::info!(
            target: "pellucid::edge::spa",
            "PELLUCID_WEBVIEW_DIST unset and /srv/webview missing — SPA fallback off"
        );
    }

    Ok((app, pool))
}

/// Tower service that wraps the SPA `ServeDir` fallback so requests
/// whose `Host:` header begins with `api_host_prefix` get a `404
/// Not Found` instead of `index.html`. Implements SPEC-001 §17's
/// `api.worldmonitor.app` rule — the binary serves both the apex
/// (SPA) and the api-prefixed hostname (API only), but only one
/// shape of fallback per request.
///
/// Inner type is the concrete `ServeDir<SetStatus<ServeFile>>` —
/// `tower_http::services::ServeDir::not_found_service(ServeFile)`
/// internally wraps the not-found service in `SetStatus` so it
/// always responds with `200 OK` rather than the file's natural
/// `404`. We carry that exact type through the field so callers
/// don't need to BoxClone-erase it.
#[derive(Clone)]
struct HostAwareSpaFallback {
    inner: tower_http::services::ServeDir<
        tower_http::set_status::SetStatus<tower_http::services::ServeFile>,
    >,
    api_host_prefix: String,
}

impl Service<Request<Body>> for HostAwareSpaFallback {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // `ServeDir`'s own `poll_ready` is `Poll::Ready(Ok(()))`
        // unconditionally; mirror that so we don't have to drag a
        // `&mut self.inner` borrow into the future.
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // `Host` may be absent for HTTP/1.0 requests or some test
        // harnesses; treat that as "not the API host" and let the
        // SPA serve. An empty `api_host_prefix` is normalised to
        // `None` in `Config::parse`, so the `starts_with` check
        // can't accidentally match every request here.
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let prefix = self.api_host_prefix.clone();
        let inner = self.inner.clone();
        Box::pin(async move {
            if host.starts_with(&prefix) {
                Ok((StatusCode::NOT_FOUND, "not found").into_response())
            } else {
                // `ServeDir`'s `Service::Error` is `Infallible`, so
                // the `Err` branch is statically unreachable. Use
                // `.map` to lift `Result<Response<_>, Infallible>`
                // → `Result<Response, Infallible>` without a panic
                // path.
                inner.oneshot(req).await.map(IntoResponse::into_response)
            }
        })
    }
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
