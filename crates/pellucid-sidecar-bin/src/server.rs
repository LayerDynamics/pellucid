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
pub fn build_router(tokens: Arc<TokenSet>) -> Router {
    let app_state = SharedAppState::new();
    let api = Router::new()
        .route("/api/echo", get(echo_get).post(echo_post))
        .layer(from_fn_with_state(tokens, require_bearer));
    Router::new()
        .route("/healthz", get(healthz))
        .merge(api)
        .with_state(app_state)
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

/// Bind to `127.0.0.1:0`, mount the router around `tokens`, and start
/// serving on a background task. Returns the bound port and a handle
/// the caller can abort to stop the server.
pub async fn serve_on_random_port(
    tokens: Arc<TokenSet>,
) -> Result<ServerHandle, ServerLaunchError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(ServerLaunchError::Bind)?;
    let addr: SocketAddr = listener.local_addr().map_err(ServerLaunchError::LocalAddr)?;
    let port = addr.port();
    let app = build_router(tokens);
    let task = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service()).await
    });
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
        let mut router = build_router(tokens);
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
        let mut router = build_router(tokens);
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
        let mut router = build_router(tokens);
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
        let mut router = build_router(tokens);
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
        let mut router = build_router(tokens);
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
            assert_eq!(
                resp.status().as_u16(),
                200,
                "token {tok} must be accepted"
            );
        }
    }

    #[tokio::test]
    async fn serve_on_random_port_returns_listening_port_and_serves_healthz() {
        let tokens = TokenSet::new("a".into());
        let handle = serve_on_random_port(tokens).await.unwrap();
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
