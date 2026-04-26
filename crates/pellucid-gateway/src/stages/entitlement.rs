//! Stage 7 — Entitlement check (H2 fix-aware).
//!
//! Calls the configured [`EntitlementChecker`] with the caller's
//! identity + the route's [`RequiredTier`]. Three outcomes:
//!
//! - `Allow` → updates the threaded [`ClientIdentity`] / [`ApiIdentity`]
//!   with the effective tier and continues.
//! - `Deny` → 403 + `entitlement_forbidden`.
//! - `UpstreamDown` → 503 + `Retry-After` (the H2 fix lands at T2.3,
//!   but the wiring is here so the stage cannot accidentally collapse
//!   `UpstreamDown` into `Deny`).
//!
//! Anonymous routes (RequiredTier == Tier::Anonymous) skip the check
//! entirely.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error_mapper::GatewayError;
use crate::identity::{ApiIdentity, ClientIdentity};
use crate::stages::tier_gate::RequiredTier;
use crate::traits::{EntitlementChecker, EntitlementDecision, Tier};

/// Per-stage state.
#[derive(Clone, Debug)]
pub struct EntitlementState(pub Arc<dyn EntitlementChecker>);

/// Middleware: entitlement check.
pub async fn entitlement(
    State(state): State<EntitlementState>,
    mut request: Request,
    next: Next,
) -> Response {
    let required = request
        .extensions()
        .get::<RequiredTier>()
        .copied()
        .map(|t| t.0)
        .unwrap_or(Tier::Anonymous);

    if required.rank() <= Tier::Anonymous.rank() {
        return next.run(request).await;
    }

    let user_key = if let Some(c) = request.extensions().get::<ClientIdentity>() {
        format!("clerk:{}", c.user_id)
    } else if let Some(a) = request.extensions().get::<ApiIdentity>() {
        format!("apikey:{}", a.identity)
    } else {
        // Should have been caught by stages 5/6, but if not, fail
        // closed: anonymous on a tier-gated route is a 401.
        return GatewayError::ClerkUnauthorized.into_response();
    };

    match state.0.check(&user_key, required).await {
        EntitlementDecision::Allow { effective_tier } => {
            // Update threaded identities so stage 11+ can read the
            // resolved tier.
            if let Some(c) = request.extensions_mut().get_mut::<ClientIdentity>() {
                c.tier = Some(effective_tier);
            }
            // For ApiIdentity we already had a tier from stage 6;
            // the entitlement check only confirms it satisfies the
            // route. No mutation needed.
            next.run(request).await
        }
        EntitlementDecision::Deny { .. } => GatewayError::EntitlementForbidden.into_response(),
        EntitlementDecision::UpstreamDown { retry_after_secs } => {
            GatewayError::EntitlementUpstreamDown { retry_after_secs }.into_response()
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::{AlwaysAllowEntitlement, AlwaysDenyEntitlement};
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request as AxumRequest, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::{Extension, Router};
    use tower::util::ServiceExt;

    #[derive(Debug)]
    struct UpstreamDownChecker;

    #[async_trait]
    impl EntitlementChecker for UpstreamDownChecker {
        async fn check(&self, _user: &str, _required: Tier) -> EntitlementDecision {
            EntitlementDecision::UpstreamDown { retry_after_secs: 30 }
        }
    }

    async fn report(Extension(c): Extension<ClientIdentity>) -> String {
        format!("{:?}", c.tier)
    }

    fn router(checker: Arc<dyn EntitlementChecker>, required: Tier) -> Router {
        Router::new()
            .route("/x", get(report))
            .layer(from_fn_with_state(EntitlementState(checker), entitlement))
            .layer(axum::middleware::from_fn(move |mut req: Request, next: Next| {
                let r = required;
                async move {
                    req.extensions_mut().insert(RequiredTier(r));
                    req.extensions_mut().insert(ClientIdentity {
                        user_id: "u".into(),
                        session_id: "s".into(),
                        tier: None,
                    });
                    next.run(req).await
                }
            }))
    }

    #[tokio::test]
    async fn anonymous_route_skipped() {
        let r = router(Arc::new(AlwaysDenyEntitlement), Tier::Anonymous);
        let resp = r
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_ne!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn allow_inserts_effective_tier_into_extensions() {
        let r = router(Arc::new(AlwaysAllowEntitlement), Tier::Free);
        let resp = r
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        assert_eq!(&body[..], b"Some(Tier2)");
    }

    #[tokio::test]
    async fn deny_returns_403_with_entitlement_code() {
        let r = router(Arc::new(AlwaysDenyEntitlement), Tier::Tier1);
        let resp = r
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            resp.headers()
                .get(crate::error_mapper::GATEWAY_ERROR_CODE_HEADER)
                .unwrap(),
            "entitlement_forbidden"
        );
    }

    #[tokio::test]
    async fn upstream_down_returns_503_with_retry_after() {
        let r = router(Arc::new(UpstreamDownChecker), Tier::Tier2);
        let resp = r
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
        assert_eq!(
            resp.headers()
                .get(crate::error_mapper::GATEWAY_ERROR_CODE_HEADER)
                .unwrap(),
            "entitlement_upstream_down"
        );
    }
}
