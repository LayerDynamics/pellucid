//! Stage 5 — Clerk session validation.
//!
//! For routes whose [`RequiredTier`] is greater than `Anonymous`, this
//! stage extracts the bearer token from the `Authorization` header and
//! consults the configured [`ClerkVerifier`]. Successful verification
//! inserts a [`ClientIdentity`] into the request extensions for the
//! entitlement stage. Missing / malformed / expired tokens return 401
//! immediately.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error_mapper::GatewayError;
use crate::identity::{ClientIdentity, RequestIdentity};
use crate::stages::tier_gate::RequiredTier;
use crate::traits::{ClerkVerifier, Tier};

/// Per-stage state.
#[derive(Clone, Debug)]
pub struct ClerkSessionState(pub Arc<dyn ClerkVerifier>);

/// Middleware: validate Clerk session for tier-gated routes.
pub async fn clerk_session(
    State(state): State<ClerkSessionState>,
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

    let token = match extract_bearer(&request) {
        Some(t) => t,
        None => {
            // No `Authorization: Bearer` header. Defer the auth-OR
            // decision to stage 6 (API key) — if that stage also
            // finds no credential it will produce the final 401.
            return next.run(request).await;
        }
    };

    match state.0.verify(&token).await {
        Ok(claims) => {
            let identity = ClientIdentity {
                user_id: claims.user_id,
                session_id: claims.session_id,
                tier: None,
            };
            // Mutate the threaded RequestIdentity envelope so stage 7
            // can read the caller through one source of truth.
            if let Some(req_id) = request.extensions_mut().get_mut::<RequestIdentity>() {
                req_id.clerk = Some(identity.clone());
            }
            // Keep ClientIdentity standalone too so stage 7's tier
            // mutation path (and the stage's own unit tests) still
            // see it directly.
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        Err(_) => GatewayError::ClerkUnauthorized.into_response(),
    }
}

fn extract_bearer(request: &Request) -> Option<String> {
    let value = request.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::to_string)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::{ClerkClaims, ClerkVerifyError};
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request as AxumRequest, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::{Extension, Router};
    use tower::util::ServiceExt;

    #[derive(Debug)]
    struct AcceptingVerifier;

    #[async_trait]
    impl ClerkVerifier for AcceptingVerifier {
        async fn verify(&self, token: &str) -> Result<ClerkClaims, ClerkVerifyError> {
            if token == "good-token" {
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

    async fn echo_user(Extension(c): Extension<ClientIdentity>) -> String {
        c.user_id
    }

    fn router(verifier: Arc<dyn ClerkVerifier>, required: Tier) -> Router {
        let state = ClerkSessionState(verifier);
        Router::new()
            .route("/x", get(echo_user))
            .layer(from_fn_with_state(state, clerk_session))
            .layer(axum::middleware::from_fn(move |mut req: Request, next: Next| {
                let r = required;
                async move {
                    req.extensions_mut().insert(RequiredTier(r));
                    next.run(req).await
                }
            }))
    }

    #[tokio::test]
    async fn anonymous_route_passes_without_token() {
        let r = router(Arc::new(AcceptingVerifier), Tier::Anonymous);
        let resp = r
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Anonymous routes do not require Clerk; but our test
        // handler still requires the extension. So the route returns
        // 500 (extension missing) rather than 401. Either non-401 is
        // acceptable proof that the stage did not reject.
        assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn tier_gated_route_without_token_defers_to_next_stage() {
        // Auth-OR contract: clerk_session does not 401 on its own
        // when there is no Authorization header. The auth-OR
        // decision is finalised by stage 6 (API key). Here, with
        // only clerk_session in the chain, the request passes
        // through and reaches the handler — which in turn fails
        // because no `ClientIdentity` was inserted. We therefore
        // assert "not 401" rather than a specific status.
        let r = router(Arc::new(AcceptingVerifier), Tier::Free);
        let resp = r
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn tier_gated_route_with_invalid_token_returns_401() {
        let r = router(Arc::new(AcceptingVerifier), Tier::Free);
        let resp = r
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .header("authorization", "Bearer bad")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn tier_gated_route_with_valid_token_inserts_identity() {
        let r = router(Arc::new(AcceptingVerifier), Tier::Free);
        let resp = r
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .header("authorization", "Bearer good-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        assert_eq!(&body[..], b"user-1");
    }

    #[tokio::test]
    async fn non_bearer_authorization_header_defers_to_next_stage() {
        // "Basic ..." is not Clerk auth — clerk_session leaves the
        // decision to stage 6 instead of 401-ing on its own.
        let r = router(Arc::new(AcceptingVerifier), Tier::Free);
        let resp = r
            .oneshot(
                AxumRequest::builder()
                    .uri("/x")
                    .header("authorization", "Basic something")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
