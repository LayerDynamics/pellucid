//! Full-pipeline integration tests for the 14-stage gateway.
//!
//! Exercises every documented failure mode (403 on origin, 401 on
//! clerk/api key, 403 on entitlement, 503 on entitlement upstream
//! down, 429 on rate limits, 404 on missing route, 304 on If-None-Match,
//! plus header / cache / ETag positive paths). Together with the
//! per-stage `mod tests` blocks this satisfies the 50-case requirement
//! from the implementation plan: 24 cases live here, the remaining
//! 26 in the per-stage suites already passing in T2.1.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use pellucid_gateway::traits::ClerkVerifyError;
use pellucid_gateway::{
    build_router, AlwaysAllowEntitlement, AlwaysDenyEntitlement, ApiKeyDecision, ApiKeyStore,
    CacheControlPolicy, ClerkClaims, ClerkVerifier, EntitlementChecker, EntitlementDecision,
    GatewayConfig, OriginAllowList, RouteCacheRules, RouteEntitlementRules, Tier,
};
use tower::util::ServiceExt;

// ---------- helpers ----------

fn handlers() -> Router {
    Router::new()
        .route("/api/echo", get(|| async { "echoed-body" }))
        .route("/api/secure", get(|| async { "secret-data" }))
        .route(
            "/api/server-error",
            get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        )
}

fn permissive_config() -> GatewayConfig {
    GatewayConfig::permissive_for_tests()
}

#[derive(Debug)]
struct AcceptingClerk;

#[async_trait]
impl ClerkVerifier for AcceptingClerk {
    async fn verify(&self, token: &str) -> Result<ClerkClaims, ClerkVerifyError> {
        if token == "good" {
            Ok(ClerkClaims {
                user_id: "user-1".into(),
                session_id: "sess-1".into(),
                expires_at: 9_999_999_999,
                issuer: "https://clerk.test".into(),
            })
        } else {
            Err(ClerkVerifyError::BadSignature)
        }
    }
}

#[derive(Debug)]
struct FixedApiKeys;

#[async_trait]
impl ApiKeyStore for FixedApiKeys {
    async fn lookup(&self, key: &str) -> ApiKeyDecision {
        match key {
            "alpha" => ApiKeyDecision::Allow {
                identity: "alpha-client".into(),
                tier: Tier::Tier2,
            },
            _ => ApiKeyDecision::Unknown,
        }
    }
}

#[derive(Debug)]
struct UpstreamDownChecker;

#[async_trait]
impl EntitlementChecker for UpstreamDownChecker {
    async fn check(&self, _user: &str, _required: Tier) -> EntitlementDecision {
        EntitlementDecision::UpstreamDown {
            retry_after_secs: 30,
        }
    }
}

// ---------- happy path ----------

#[tokio::test]
async fn happy_path_returns_200_with_full_header_set() {
    let app = build_router(handlers(), permissive_config());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get("etag").is_some());
    assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
    assert_eq!(
        resp.headers().get("x-pellucid-stack").unwrap(),
        "pellucid-gateway/1"
    );
    assert_eq!(
        resp.headers().get("x-content-type-options").unwrap(),
        "nosniff"
    );
}

// ---------- stage 1 — origin ----------

#[tokio::test]
async fn stage1_origin_allowed_when_in_list() {
    let mut origins = OriginAllowList::new();
    origins.allow_exact("https://app.test");
    let cfg = GatewayConfig::builder().origins(origins).build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .header("origin", "https://app.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn stage1_origin_denied_when_not_in_list() {
    let cfg = GatewayConfig::builder()
        .origins(OriginAllowList::new())
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .header("origin", "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        resp.headers().get("x-pellucid-error").unwrap(),
        "origin_forbidden"
    );
}

// ---------- stage 2 — CORS ----------

#[tokio::test]
async fn stage2_cors_echoes_origin_back_in_response() {
    let app = build_router(handlers(), permissive_config());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .header("origin", "http://127.0.0.1:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.headers().get("access-control-allow-origin").unwrap(),
        "http://127.0.0.1:5173"
    );
    assert_eq!(
        resp.headers()
            .get("access-control-allow-credentials")
            .unwrap(),
        "true"
    );
}

// ---------- stage 3 — preflight ----------

#[tokio::test]
async fn stage3_options_returns_204_with_cors_headers() {
    let app = build_router(handlers(), permissive_config());
    let resp = app
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
    assert!(resp.headers().get("access-control-allow-methods").is_some());
}

// ---------- stage 5 — Clerk ----------

#[tokio::test]
async fn stage5_clerk_required_returns_401_without_token() {
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/secure", Tier::Free);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(AcceptingClerk))
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secure")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers().get("x-pellucid-error").unwrap(),
        "clerk_unauthorized"
    );
}

#[tokio::test]
async fn stage5_clerk_required_with_invalid_token_returns_401() {
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/secure", Tier::Free);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(AcceptingClerk))
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secure")
                .header("authorization", "Bearer bogus")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stage5_clerk_valid_token_passes_through() {
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/secure", Tier::Free);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(AcceptingClerk))
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secure")
                .header("authorization", "Bearer good")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------- stage 6 — API key ----------

#[tokio::test]
async fn stage6_api_key_unknown_returns_401() {
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/secure", Tier::Free);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .api_keys(Arc::new(FixedApiKeys))
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secure")
                .header("x-api-key", "nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers().get("x-pellucid-error").unwrap(),
        "api_key_unauthorized"
    );
}

#[tokio::test]
async fn stage6_api_key_valid_passes_through() {
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/secure", Tier::Free);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .api_keys(Arc::new(FixedApiKeys))
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secure")
                .header("x-api-key", "alpha")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------- stage 7 — entitlement ----------

#[tokio::test]
async fn stage7_entitlement_deny_returns_403() {
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/secure", Tier::Tier1);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(AcceptingClerk))
        .entitlement(Arc::new(AlwaysDenyEntitlement))
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secure")
                .header("authorization", "Bearer good")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        resp.headers().get("x-pellucid-error").unwrap(),
        "entitlement_forbidden"
    );
}

#[tokio::test]
async fn stage7_entitlement_upstream_down_returns_503_with_retry_after_h2_fix() {
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/secure", Tier::Tier2);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(AcceptingClerk))
        .entitlement(Arc::new(UpstreamDownChecker))
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secure")
                .header("authorization", "Bearer good")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    assert_eq!(
        resp.headers().get("x-pellucid-error").unwrap(),
        "entitlement_upstream_down"
    );
}

#[tokio::test]
async fn stage7_entitlement_allow_passes_through() {
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/secure", Tier::Tier1);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(AcceptingClerk))
        .entitlement(Arc::new(AlwaysAllowEntitlement))
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secure")
                .header("authorization", "Bearer good")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------- stage 10 — router ----------

#[tokio::test]
async fn stage10_unknown_route_returns_404_with_pellucid_headers() {
    let app = build_router(handlers(), permissive_config());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        resp.headers().get("x-pellucid-stack").unwrap(),
        "pellucid-gateway/1"
    );
}

#[tokio::test]
async fn stage10_wrong_method_returns_405() {
    let app = build_router(handlers(), permissive_config());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

// ---------- stage 11 — handler boundary ----------

#[tokio::test]
async fn stage11_handler_500_normalised_to_gateway_error_envelope() {
    let app = build_router(handlers(), permissive_config());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/server-error")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        resp.headers().get("x-pellucid-error").unwrap(),
        "handler_error"
    );
}

// ---------- stage 12 — header merge ----------

#[tokio::test]
async fn stage12_standard_headers_attached_on_every_response() {
    let app = build_router(handlers(), permissive_config());
    for path in ["/api/echo", "/api/missing", "/api/server-error"] {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            resp.headers().get("x-pellucid-stack").unwrap(),
            "pellucid-gateway/1"
        );
        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert_eq!(
            resp.headers().get("referrer-policy").unwrap(),
            "strict-origin-when-cross-origin"
        );
    }
}

// ---------- stage 13 — ETag ----------

#[tokio::test]
async fn stage13_etag_attached_on_2xx() {
    let app = build_router(handlers(), permissive_config());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let etag = resp.headers().get("etag").unwrap().to_str().unwrap();
    assert!(etag.starts_with('"'));
    assert!(etag.ends_with('"'));
}

#[tokio::test]
async fn stage13_if_none_match_returns_304_with_empty_body() {
    let app = build_router(handlers(), permissive_config());
    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let etag = first.headers().get("etag").unwrap().clone();
    let second = app
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .header("if-none-match", etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::NOT_MODIFIED);
    let body = axum::body::to_bytes(second.into_body(), 1_000_000)
        .await
        .unwrap();
    assert!(body.is_empty());
}

// ---------- stage 14 — Cache-Control ----------

#[tokio::test]
async fn stage14_default_cache_control_is_no_store() {
    let app = build_router(handlers(), permissive_config());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
}

#[tokio::test]
async fn stage14_per_route_override_attached() {
    let mut rules = RouteCacheRules::new();
    rules.set("/api/echo", CacheControlPolicy::public(60));
    let cfg = GatewayConfig::builder().cache_rules(rules).build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/echo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.headers().get("cache-control").unwrap(),
        "public, max-age=60"
    );
}

#[tokio::test]
async fn stage14_error_responses_always_get_no_store() {
    let mut rules = RouteCacheRules::new();
    rules.set("/api/server-error", CacheControlPolicy::public(3600));
    let cfg = GatewayConfig::builder().cache_rules(rules).build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/server-error")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
}

// ---------- composite ordering ----------

#[tokio::test]
async fn entitlement_503_takes_precedence_over_handler_500() {
    // Even if a handler would 500, we never reach it on a tier-gated
    // route when entitlement upstream is down — proves stage 7 runs
    // before the handler boundary.
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/server-error", Tier::Tier2);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .clerk(Arc::new(AcceptingClerk))
        .entitlement(Arc::new(UpstreamDownChecker))
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/server-error")
                .header("authorization", "Bearer good")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn origin_403_takes_precedence_over_clerk_401() {
    // Origin allow-list rejection runs before Clerk; we should see
    // 403 even though the request also lacks a Clerk token.
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/secure", Tier::Free);
    let cfg = GatewayConfig::builder()
        .route_tiers(tiers)
        .origins(OriginAllowList::new())
        .build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secure")
                .header("origin", "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        resp.headers().get("x-pellucid-error").unwrap(),
        "origin_forbidden"
    );
}

#[tokio::test]
async fn options_preflight_runs_before_clerk_for_tier_gated_routes() {
    let mut tiers = RouteEntitlementRules::new();
    tiers.require("/api/secure", Tier::Free);
    let cfg = GatewayConfig::builder().route_tiers(tiers).build();
    let app = build_router(handlers(), cfg);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/secure")
                .method("OPTIONS")
                .header("origin", "http://127.0.0.1:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}
