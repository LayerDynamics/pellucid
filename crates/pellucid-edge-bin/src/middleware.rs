//! Edge-bin-specific middleware.
//!
//! Shipped today: a minimal `/healthz` route that the deploy
//! supervisor (Fly health monitor, Docker `restart: on-failure`)
//! pings to confirm the binary is up. Returns 200 + `{ "ok":
//! true, "version": "<crate version>" }`.
//!
//! Bot-UA filter, social-preview UA allowlist, and CSP headers
//! land in T3.12's middleware expansion (per spec §11.4) on top
//! of this same surface.

use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

/// Healthcheck path. Bound on the same port as the gateway
/// router so deploy probes hit a single endpoint.
pub const HEALTHCHECK_PATH: &str = "/healthz";

/// Healthcheck response shape — pinned for the Playwright +
/// `boot.rs` integration tests to assert on.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HealthcheckBody {
    /// Always `true` from this surface — the deploy probe
    /// branches on the HTTP status, not the body. Body fields
    /// are diagnostic.
    pub ok: bool,
    /// Crate version string, so a probe can verify the deploy
    /// is the version the operator just shipped.
    pub version: String,
}

/// Build the healthcheck router. Mounted alongside the gateway
/// in the binary's `main`.
pub fn healthcheck_router() -> Router {
    Router::new().route(HEALTHCHECK_PATH, get(healthcheck))
}

async fn healthcheck() -> Json<HealthcheckBody> {
    Json(HealthcheckBody {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn healthz_returns_200_and_ok_body() {
        let app = healthcheck_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(HEALTHCHECK_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: HealthcheckBody = serde_json::from_slice(&body).unwrap();
        assert!(parsed.ok);
        assert_eq!(parsed.version, env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn healthz_path_is_pinned_for_supervisor_probes() {
        // Unit guard: the path is stable, so a Dockerfile
        // healthcheck wired to /healthz keeps working across
        // refactors.
        assert_eq!(HEALTHCHECK_PATH, "/healthz");
    }
}
