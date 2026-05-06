//! Stage 4 — Tier gate.
//!
//! Reads the configured [`RouteEntitlementRules`] and inserts a
//! [`RequiredTier`] into the request extensions for downstream stages
//! (5/6/7) to consult. This is the only stage in the chain that
//! decides whether a route is "tier-gated" at all — stage 5 then
//! short-circuits 401 if `RequiredTier > Tier::Anonymous` and no
//! Clerk session is present.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::RouteEntitlementRules;
use crate::traits::Tier;

/// Required tier resolved for the current request, threaded through
/// `request.extensions()`.
#[derive(Clone, Copy, Debug)]
pub struct RequiredTier(pub Tier);

/// Middleware: insert the required tier into the request extensions.
pub async fn tier_gate(
    State(rules): State<Arc<RouteEntitlementRules>>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let required = rules.required_for(&path);
    request.extensions_mut().insert(RequiredTier(required));
    next.run(request).await
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as AxumRequest, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::{Extension, Router};
    use tower::util::ServiceExt;

    async fn echo_required(Extension(t): Extension<RequiredTier>) -> String {
        format!("{:?}", t.0)
    }

    fn router(rules: Arc<RouteEntitlementRules>) -> Router {
        Router::new()
            .route("/x", get(echo_required))
            .route("/y", get(echo_required))
            .layer(from_fn_with_state(rules, tier_gate))
    }

    #[tokio::test]
    async fn unmapped_path_defaults_to_anonymous() {
        let rules = Arc::new(RouteEntitlementRules::new());
        let resp = router(rules)
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        assert_eq!(&body[..], b"Anonymous");
    }

    #[tokio::test]
    async fn mapped_path_resolves_to_configured_tier() {
        let mut rules = RouteEntitlementRules::new();
        rules.require("/y", Tier::Tier2);
        let resp = router(Arc::new(rules))
            .oneshot(
                AxumRequest::builder()
                    .uri("/y")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        assert_eq!(&body[..], b"Tier2");
    }

    #[tokio::test]
    async fn each_path_evaluated_independently() {
        let mut rules = RouteEntitlementRules::new();
        rules.require("/x", Tier::Free);
        rules.require("/y", Tier::Tier1);
        let r = router(Arc::new(rules));
        let a = r
            .clone()
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let b = r
            .oneshot(
                AxumRequest::builder()
                    .uri("/y")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body_a = axum::body::to_bytes(a.into_body(), 1_000_000)
            .await
            .unwrap();
        let body_b = axum::body::to_bytes(b.into_body(), 1_000_000)
            .await
            .unwrap();
        assert_eq!(&body_a[..], b"Free");
        assert_eq!(&body_b[..], b"Tier1");
    }
}
