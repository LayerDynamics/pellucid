//! Server bootstrap — Axum router + dynamic-port listener.
//!
//! The sidecar binds to `127.0.0.1:0`, prints `PORT=<n>` on stdout so
//! the host's [`SidecarSupervisor`](pellucid_tauri::SidecarSupervisor)
//! can capture the port, then mounts the bearer-protected `/api/echo`
//! routes plus an unauthenticated `/healthz` for diagnostics.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::middleware::from_fn_with_state;
use axum::routing::get;
use axum::Router;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use pellucid_cache::CoalesceRegistry;
use pellucid_core::vault::Vault;
use pellucid_handlers::{build_handlers, AppState};
use pellucid_ml::{build_from_vault, FromVaultError, MlEngine, VaultEngineConfig};

use crate::auth::{require_bearer, TokenSet};
use crate::echo::{echo_get, echo_post};

/// Stdout discovery prefix — must match
/// `pellucid_tauri::sidecar::PORT_LINE_PREFIX`.
pub const STDOUT_PORT_PREFIX: &str = "PORT=";

/// Errors raised while constructing or running the server.
#[derive(Debug, Error)]
pub enum ServerLaunchError {
    /// `bind(127.0.0.1:0)` failed.
    #[error("bind failed: {0}")]
    Bind(#[source] std::io::Error),
    /// Reading the bound port back from the listener failed.
    #[error("local_addr failed: {0}")]
    LocalAddr(#[source] std::io::Error),
}

/// Per-process state shared by every Axum handler.
#[derive(Clone, Debug)]
pub struct SharedAppState {
    started_at: Instant,
}

impl SharedAppState {
    /// Construct a fresh state stamped at "now".
    #[must_use]
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }

    /// Process uptime in milliseconds at the moment of the call.
    #[must_use]
    pub fn uptime_ms(&self) -> u64 {
        u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

impl Default for SharedAppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the Axum router. `/api/*` requires a bearer; `/healthz` does
/// not. Exposed publicly so integration tests can compose the same
/// router into their own server harnesses.
///
/// `handler_state` is the full `pellucid_handlers::AppState` —
/// when supplied, the sidecar mounts every handler the
/// `pellucid-edge-bin` does (intelligence, correlation, news,
/// market, …) under bearer auth so the webview can hit them via
/// `127.0.0.1:<port>` without going through `pellucid.world`.
/// When `None`, only `/api/echo` ships (the original T1.9 minimum).
pub fn build_router(tokens: Arc<TokenSet>, handler_state: Option<AppState>) -> Router {
    let app_state = SharedAppState::new();
    // Echo + healthz both need `SharedAppState`; bake it in here so
    // the resulting `Router<()>` has a stable state-erased type and
    // can merge cleanly with `build_handlers(...)` (which uses the
    // pellucid-handlers `AppState` baked in per-route).
    let echo: Router = Router::new()
        .route("/api/echo", get(echo_get).post(echo_post))
        .with_state(app_state.clone());
    let healthz_route: Router = Router::new()
        .route("/healthz", get(healthz))
        .with_state(app_state);
    let mut api: Router = echo;
    if let Some(state) = handler_state {
        api = api.merge(build_handlers(state));
    }
    let api = api.layer(from_fn_with_state(tokens, require_bearer));
    healthz_route.merge(api)
}

/// Backwards-compatible single-argument constructor used by the
/// existing token-rotation integration tests. New callers should
/// prefer [`build_router`] so they can opt into the full handler
/// surface.
pub fn build_router_echo_only(tokens: Arc<TokenSet>) -> Router {
    build_router(tokens, None)
}

/// Build a sidecar `AppState` + (optional) `MlEngine` from process
/// env. Mirrors `pellucid_edge_bin::build_app`'s wiring without the
/// router-mount step, so the desktop sidecar gets the same tier-2
/// ML endpoints that edge serves to web SaaS customers.
///
/// Reads `GROQ_API_KEY` / `HF_TOKEN` via
/// [`pellucid_core::vault::EnvVault::from_env`]. When either key is
/// absent, the engine constructor returns `Ok(None)` and the ML
/// handlers respond with 503 just like edge does — so the
/// onboarding UX is identical across desktop and web.
pub async fn build_handler_state_from_env() -> Result<AppState, ServerLaunchError> {
    use pellucid_core::vault::EnvVault;
    use pellucid_db::{open_in_memory, Pool};

    let pool: Pool = open_in_memory()
        .await
        .map_err(|e| ServerLaunchError::Bind(std::io::Error::other(e)))?;
    let mut state = AppState::new(
        pool,
        Arc::new(CoalesceRegistry::default()),
        // Aviation upstream — the desktop sidecar isn't wired to
        // aviationstack today (the desktop uses its own offline
        // cache), so install the null upstream and let any
        // aviation handler return `Ok(None)`. Web SaaS users get
        // the real client via edge-bin.
        Arc::new(NullAviation),
    );
    let vault = EnvVault::from_env();
    if let Some(engine) = build_ml_engine(&vault).await {
        state = state.with_ml(engine);
    }
    Ok(state)
}

#[derive(Debug)]
struct NullAviation;
#[async_trait::async_trait]
impl pellucid_handlers::FlightStatusUpstream for NullAviation {
    async fn fetch_flight(
        &self,
        _flight: &str,
        _date: &str,
        _origin: &str,
    ) -> Result<
        Option<pellucid_handlers::generated::aviation::v1::FlightStatus>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        Ok(None)
    }
}

async fn build_ml_engine(vault: &dyn Vault) -> Option<Arc<dyn MlEngine>> {
    match build_from_vault(vault, &VaultEngineConfig::default()).await {
        Ok(engine) => {
            tracing::info!(
                target: "pellucid::sidecar::ml",
                "MlEngine constructed from sidecar env — intelligence handlers active"
            );
            Some(engine)
        }
        Err(FromVaultError::MissingKey(name)) => {
            tracing::warn!(
                target: "pellucid::sidecar::ml",
                missing = name,
                "MlEngine not constructed — sidecar's intelligence endpoints will return 503 until {name} is set"
            );
            None
        }
        Err(other) => {
            tracing::warn!(
                target: "pellucid::sidecar::ml",
                error = %other,
                "MlEngine construction failed — sidecar's intelligence endpoints will return 503"
            );
            None
        }
    }
}

async fn healthz() -> &'static str {
    "ok"
}

/// Handle returned by [`serve_on_random_port`].
#[derive(Debug)]
pub struct ServerHandle {
    /// Bound port the listener is accepting connections on.
    pub port: u16,
    /// `JoinHandle` of the background task running the server. Abort
    /// it to stop the server.
    pub task: JoinHandle<std::io::Result<()>>,
}

/// Bind to `127.0.0.1:0`, mount the router around `tokens` and the
/// supplied `AppState` (or echo-only when `None`), and start
/// serving on a background task. Returns the bound port and a
/// handle the caller can abort to stop the server.
pub async fn serve_on_random_port(
    tokens: Arc<TokenSet>,
    handler_state: Option<AppState>,
) -> Result<ServerHandle, ServerLaunchError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(ServerLaunchError::Bind)?;
    let addr: SocketAddr = listener
        .local_addr()
        .map_err(ServerLaunchError::LocalAddr)?;
    let port = addr.port();
    let app = build_router(tokens, handler_state);
    let task = tokio::spawn(async move { axum::serve(listener, app.into_make_service()).await });
    Ok(ServerHandle { port, task })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn shared_state_uptime_advances() {
        let s = SharedAppState::new();
        let a = s.uptime_ms();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = s.uptime_ms();
        assert!(b >= a);
    }

    #[test]
    fn stdout_port_prefix_matches_supervisor() {
        // pellucid-tauri's SidecarSupervisor strips `PORT=` from the
        // first matching stdout line. Both crates must agree.
        assert_eq!(STDOUT_PORT_PREFIX, "PORT=");
    }

    #[tokio::test]
    async fn build_router_returns_a_router_that_responds_to_healthz() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::util::ServiceExt;

        let tokens = TokenSet::new("any".into());
        let mut router = build_router(tokens, None);
        let resp = router
            .as_service()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn echo_route_rejects_request_without_bearer_with_401() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::util::ServiceExt;

        let tokens = TokenSet::new("tok".into());
        let mut router = build_router(tokens, None);
        let resp = router
            .as_service()
            .oneshot(
                Request::builder()
                    .uri("/api/echo")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 401);
    }

    #[tokio::test]
    async fn echo_route_with_valid_bearer_returns_200_and_echoes_message() {
        use axum::body::{to_bytes, Body};
        use axum::http::Request;
        use tower::util::ServiceExt;

        let tokens = TokenSet::new("good".into());
        let mut router = build_router(tokens, None);
        let resp = router
            .as_service()
            .oneshot(
                Request::builder()
                    .uri("/api/echo?message=hello")
                    .header("authorization", "Bearer good")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        let body_bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(parsed["message"].as_str(), Some("hello"));
    }

    #[tokio::test]
    async fn echo_route_accepts_post_body_round_trip() {
        use axum::body::{to_bytes, Body};
        use axum::http::Request;
        use tower::util::ServiceExt;

        let tokens = TokenSet::new("good".into());
        let mut router = build_router(tokens, None);
        let req_body = serde_json::json!({
            "message": "from-post",
            "payload": { "k": 1, "v": [1,2,3] },
        });
        let resp = router
            .as_service()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/echo")
                    .header("authorization", "Bearer good")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&req_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        let body_bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(parsed["message"].as_str(), Some("from-post"));
        assert_eq!(parsed["payload"]["k"].as_i64(), Some(1));
    }

    #[tokio::test]
    async fn previous_token_accepted_after_rotation() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::util::ServiceExt;

        let tokens = TokenSet::new("v1".into());
        tokens.rotate_to("v2".into());
        let mut router = build_router(tokens, None);
        for tok in ["v1", "v2"] {
            let resp = router
                .as_service()
                .oneshot(
                    Request::builder()
                        .uri("/api/echo")
                        .header("authorization", format!("Bearer {tok}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status().as_u16(), 200, "token {tok} must be accepted");
        }
    }

    #[tokio::test]
    async fn serve_on_random_port_returns_listening_port_and_serves_healthz() {
        let tokens = TokenSet::new("a".into());
        let handle = serve_on_random_port(tokens, None).await.unwrap();
        assert!(handle.port >= 1, "port must be assigned");
        let body = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{}/healthz", handle.port))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body.as_str(), "ok");
        handle.task.abort();
    }
}
