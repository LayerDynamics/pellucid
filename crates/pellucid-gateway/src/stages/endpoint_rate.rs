//! Stage 8 — Endpoint rate limit.
//!
//! Calls `pellucid_cache::rate_limit::check_rate_limit` with the
//! per-route configuration resolved from [`RouteRateLimitRules`]. The
//! check writes through all three buckets (endpoint, global,
//! aggregate) — but stage 8's responsibility is reporting the result
//! using the endpoint bucket's name when a deny was triggered there;
//! stage 9 maps any global / aggregate deny to its own error code.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use parking_lot::RwLock;

use pellucid_cache::rate_limit::{check_rate_limit, BucketKind, RateLimitDecision};
use pellucid_db::Pool;

use crate::config::RouteRateLimitRules;
use crate::error_mapper::GatewayError;
use crate::identity::RequestIdentity;

/// Per-stage state.
#[derive(Clone)]
pub struct EndpointRateState {
    /// Per-path overrides + default config.
    pub rules: Arc<RouteRateLimitRules>,
    /// Optional rate-limit pool. When `None`, the stage short-
    /// circuits to `Allow` (test fixture / no-database deployments).
    pub pool: Option<Arc<RwLock<Option<Pool>>>>,
}

impl std::fmt::Debug for EndpointRateState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointRateState")
            .field("has_pool", &self.pool.is_some())
            .finish()
    }
}

/// Marker inserted into request extensions when stage 8's check
/// allowed the request, so stage 9 can skip a duplicate write.
#[derive(Clone, Copy, Debug)]
pub struct RateLimitChecked;

/// Middleware: per-endpoint rate limit.
pub async fn endpoint_rate(
    State(state): State<EndpointRateState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(pool_handle) = state.pool.clone() else {
        return next.run(request).await;
    };
    let Some(pool) = pool_handle.read().clone() else {
        return next.run(request).await;
    };

    let identity = request
        .extensions()
        .get::<RequestIdentity>()
        .cloned()
        .unwrap_or_else(|| RequestIdentity::anonymous(std::net::IpAddr::from([127, 0, 0, 1])));
    let path = request.uri().path().to_string();
    let cfg = state.rules.for_path(&path);
    let ip = identity.ip.to_string();

    match check_rate_limit(&pool, &path, &ip, &cfg).await {
        Ok(RateLimitDecision::Allowed { .. }) => {
            request.extensions_mut().insert(RateLimitChecked);
            next.run(request).await
        }
        Ok(RateLimitDecision::Denied {
            bucket: BucketKind::Endpoint,
            retry_after_ms,
        }) => GatewayError::RateLimited {
            bucket: "endpoint",
            retry_after_secs: ((retry_after_ms / 1000).max(1)) as u32,
        }
        .into_response(),
        Ok(RateLimitDecision::Denied {
            bucket: BucketKind::Global,
            retry_after_ms,
        }) => GatewayError::RateLimited {
            bucket: "global",
            retry_after_secs: ((retry_after_ms / 1000).max(1)) as u32,
        }
        .into_response(),
        Ok(RateLimitDecision::Denied {
            bucket: BucketKind::Aggregate,
            retry_after_ms,
        }) => GatewayError::RateLimited {
            bucket: "aggregate",
            retry_after_secs: ((retry_after_ms / 1000).max(1)) as u32,
        }
        .into_response(),
        Err(err) => {
            tracing::error!(target: "pellucid::gateway", "rate-limit check failed: {err}");
            // Fail open on database errors — better to serve a request
            // than to silently 429 every caller during a SQLite blip.
            request.extensions_mut().insert(RateLimitChecked);
            next.run(request).await
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as AxumRequest, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::Router;
    use tower::util::ServiceExt;

    fn router_no_pool() -> Router {
        let state = EndpointRateState {
            rules: Arc::new(RouteRateLimitRules::default()),
            pool: None,
        };
        Router::new()
            .route("/x", get(|| async { "ok" }))
            .layer(from_fn_with_state(state, endpoint_rate))
    }

    #[tokio::test]
    async fn no_pool_short_circuits_to_allow() {
        let resp = router_no_pool()
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn pool_present_with_no_db_short_circuits_to_allow() {
        let state = EndpointRateState {
            rules: Arc::new(RouteRateLimitRules::default()),
            pool: Some(Arc::new(RwLock::new(None))),
        };
        let app = Router::new()
            .route("/x", get(|| async { "ok" }))
            .layer(from_fn_with_state(state, endpoint_rate));
        let resp = app
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn endpoint_rate_state_debug_does_not_leak_pool() {
        let state = EndpointRateState {
            rules: Arc::new(RouteRateLimitRules::default()),
            pool: None,
        };
        assert_eq!(
            format!("{state:?}"),
            "EndpointRateState { has_pool: false }"
        );
    }
}
