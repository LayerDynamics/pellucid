//! Stage 6 — API key validation.
//!
//! For routes whose [`RequiredTier`] is greater than `Anonymous` AND
//! the request carries an `x-api-key` header (instead of a Clerk
//! bearer), this stage looks up the key in the configured
//! [`ApiKeyStore`] and either inserts an [`ApiIdentity`] or returns
//! 401. Routes are auth-OR: stage 5 may have already inserted a
//! Clerk identity, in which case this stage is a no-op.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::HeaderName;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error_mapper::GatewayError;
use crate::identity::{ApiIdentity, ClientIdentity};
use crate::stages::tier_gate::RequiredTier;
use crate::traits::{ApiKeyDecision, ApiKeyStore, Tier};

/// Header name for API key auth.
pub const API_KEY_HEADER: HeaderName = HeaderName::from_static("x-api-key");

/// Per-stage state.
#[derive(Clone, Debug)]
pub struct ApiKeyState(pub Arc<dyn ApiKeyStore>);

/// Middleware: API key validation. No-op for anonymous routes and for
/// requests already carrying a Clerk identity.
pub async fn api_key(
    State(state): State<ApiKeyState>,
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

    if request.extensions().get::<ClientIdentity>().is_some() {
        // Clerk auth already passed.
        return next.run(request).await;
    }

    let key = match request.headers().get(&API_KEY_HEADER).and_then(|v| v.to_str().ok()) {
        Some(k) if !k.is_empty() => k.to_string(),
        _ => {
            // No api key header. Stage 5 already failed (else it would
            // have inserted a ClientIdentity); produce 401.
            return GatewayError::ClerkUnauthorized.into_response();
        }
    };

    match state.0.lookup(&key).await {
        ApiKeyDecision::Allow { identity, tier } => {
            request.extensions_mut().insert(ApiIdentity { identity, tier });
            next.run(request).await
        }
        ApiKeyDecision::Revoked | ApiKeyDecision::Unknown => {
            GatewayError::ApiKeyUnauthorized.into_response()
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request as AxumRequest, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::{Extension, Router};
    use tower::util::ServiceExt;

    #[derive(Debug)]
    struct StaticStore;

    #[async_trait]
    impl ApiKeyStore for StaticStore {
        async fn lookup(&self, key: &str) -> ApiKeyDecision {
            match key {
                "alpha" => ApiKeyDecision::Allow {
                    identity: "alpha-client".into(),
                    tier: Tier::Tier2,
                },
                "revoked" => ApiKeyDecision::Revoked,
                _ => ApiKeyDecision::Unknown,
            }
        }
    }

    async fn echo_api(Extension(a): Extension<ApiIdentity>) -> String {
        format!("{}|{:?}", a.identity, a.tier)
    }

    fn router(required: Tier) -> Router {
        let state = ApiKeyState(Arc::new(StaticStore));
        Router::new()
            .route("/x", get(echo_api))
            .layer(from_fn_with_state(state, api_key))
            .layer(axum::middleware::from_fn(move |mut req: Request, next: Next| {
                let r = required;
                async move {
                    req.extensions_mut().insert(RequiredTier(r));
                    next.run(req).await
                }
            }))
    }

    #[tokio::test]
    async fn anonymous_route_skipped() {
        let r = router(Tier::Anonymous);
        let resp = r
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_key_on_tier_gated_route_returns_401() {
        let r = router(Tier::Free);
        let resp = r
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unknown_key_returns_401_with_apikey_code() {
        let r = router(Tier::Free);
        let resp = r
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .header("x-api-key", "nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers()
                .get(crate::error_mapper::GATEWAY_ERROR_CODE_HEADER)
                .unwrap(),
            "api_key_unauthorized"
        );
    }

    #[tokio::test]
    async fn revoked_key_returns_401() {
        let r = router(Tier::Free);
        let resp = r
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .header("x-api-key", "revoked")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_key_inserts_identity() {
        let r = router(Tier::Free);
        let resp = r
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .header("x-api-key", "alpha")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        assert_eq!(&body[..], b"alpha-client|Tier2");
    }
}
