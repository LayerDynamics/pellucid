//! Router composer — assembles the 14 stages around a [`HandlerSet`].
//!
//! `build_router(handlers, config)` returns a fully-wired
//! `axum::Router`. The middleware order applied here mirrors
//! SPEC-001 §8.3 exactly:
//!
//! 1. Origin allow-list           (outermost)
//! 2. CORS merge
//! 3. OPTIONS preflight
//! 4. Tier gate
//! 5. Clerk session
//! 6. API key
//! 7. Entitlement
//! 8. Endpoint rate-limit
//! 9. Global rate-limit
//! 10. Router (Axum)
//! 11. Handler error boundary
//! 12. Header merge
//! 13. ETag
//! 14. Cache-Control                (innermost = closest to response)

use std::sync::Arc;

use axum::middleware::from_fn_with_state;
use axum::Router;

use crate::config::GatewayConfig;
use crate::identity::RequestIdentity;
use crate::stages::{
    api_key::ApiKeyState, cache_control::CacheControlState, clerk_session::ClerkSessionState,
    cors::CorsState, endpoint_rate::EndpointRateState, entitlement::EntitlementState,
    origin::OriginAllowListState, tier_gate::TierGateState,
};
use crate::stages::{
    api_key, cache_control, clerk_session, cors_merge, endpoint_rate, entitlement, etag,
    global_rate, handler_boundary, header_merge, options_preflight, origin_allow_list, tier_gate,
};

/// Bundle of routes and the per-deployment gateway config. The
/// gateway is constructed once and shared by `pellucid-edge-bin` and
/// `pellucid-sidecar-bin`.
#[derive(Clone, Debug)]
pub struct GatewayState {
    /// Config the stages read from.
    pub config: Arc<GatewayConfig>,
}

/// Trait every handler-bundle implements. The gateway treats
/// handlers as a black box producing an `axum::Router`; this trait
/// is what `pellucid-handlers` will implement at T2.5+.
pub trait HandlerSet {
    /// Convert the handler bundle into the route table the gateway
    /// will mount.
    fn into_router(self) -> Router;
}

/// Adapter so a bare `axum::Router` is itself a `HandlerSet`. Useful
/// for tests that just wire a `/echo` handler without a full bundle.
impl HandlerSet for Router {
    fn into_router(self) -> Router {
        self
    }
}

/// Compose the 14-stage pipeline around `handlers`. Returns the final
/// `axum::Router` ready to bind on a TCP listener.
pub fn build_router<H: HandlerSet>(handlers: H, config: GatewayConfig) -> Router {
    let cfg = Arc::new(config);

    let cache_state = CacheControlState(Arc::new(cfg.cache_rules.clone()));
    let cors_state = CorsState(Arc::new(cfg.cors.clone()));
    let origin_state = OriginAllowListState(Arc::new(cfg.origins.clone()));
    let tier_state = TierGateState(Arc::new(cfg.route_tiers.clone()));
    let clerk_state = ClerkSessionState(cfg.clerk.clone());
    let api_state = ApiKeyState(cfg.api_keys.clone());
    let entitlement_state = EntitlementState(cfg.entitlement.clone());
    let endpoint_state = EndpointRateState {
        rules: Arc::new(cfg.rate_limits.clone()),
        pool: cfg.rate_limit_pool.clone(),
    };

    handlers
        .into_router()
        // Innermost layers run last on the way in, first on the way
        // out. Because Axum stacks layers in reverse insertion order,
        // we apply them bottom-up so the spec's stage 1 is outermost.
        .layer(from_fn_with_state(cache_state, cache_control))
        .layer(axum::middleware::from_fn(etag))
        .layer(axum::middleware::from_fn(header_merge))
        .layer(axum::middleware::from_fn(handler_boundary))
        .layer(axum::middleware::from_fn(global_rate))
        .layer(from_fn_with_state(endpoint_state, endpoint_rate))
        .layer(from_fn_with_state(entitlement_state, entitlement))
        .layer(from_fn_with_state(api_state, api_key))
        .layer(from_fn_with_state(clerk_state, clerk_session))
        .layer(from_fn_with_state(tier_state, tier_gate))
        .layer(axum::middleware::from_fn(options_preflight))
        .layer(from_fn_with_state(cors_state, cors_merge))
        .layer(from_fn_with_state(origin_state, origin_allow_list))
        // Outside everything: thread a `RequestIdentity` so stages
        // 8 / 9 see the caller's IP without re-parsing headers.
        .layer(axum::middleware::from_fn(install_request_identity))
}

async fn install_request_identity(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if request.extensions().get::<RequestIdentity>().is_none() {
        let ip = extract_caller_ip(&request);
        request
            .extensions_mut()
            .insert(RequestIdentity::anonymous(ip));
    }
    next.run(request).await
}

fn extract_caller_ip(request: &axum::extract::Request) -> std::net::IpAddr {
    if let Some(forwarded) = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(first) = forwarded.split(',').next() {
            if let Ok(ip) = first.trim().parse() {
                return ip;
            }
        }
    }
    std::net::IpAddr::from([127, 0, 0, 1])
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::util::ServiceExt;

    fn build() -> Router {
        let routes = Router::new().route("/api/echo", get(|| async { "echoed" }));
        build_router(routes, GatewayConfig::permissive_for_tests())
    }

    #[tokio::test]
    async fn full_pipeline_returns_handler_body_on_happy_path() {
        let resp = build()
            .oneshot(Request::builder().uri("/api/echo").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("x-pellucid-stack").unwrap(), "pellucid-gateway/1");
        assert!(resp.headers().get("etag").is_some());
        assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
    }

    #[tokio::test]
    async fn router_handles_options_with_204() {
        let resp = build()
            .oneshot(
                Request::builder()
                    .uri("/api/echo")
                    .method("OPTIONS")
                    .header("origin", "http://127.0.0.1:5173")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            resp.headers().get("access-control-allow-origin").unwrap(),
            "http://127.0.0.1:5173"
        );
    }

    #[tokio::test]
    async fn router_404s_unmapped_path() {
        let resp = build()
            .oneshot(Request::builder().uri("/api/missing").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(resp.headers().get("x-pellucid-stack").unwrap(), "pellucid-gateway/1");
    }
}
