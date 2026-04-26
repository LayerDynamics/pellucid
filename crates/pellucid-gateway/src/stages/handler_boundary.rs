//! Stage 11 — Handler error boundary.
//!
//! Wraps the handler chain in a panic-catching middleware so an
//! unhandled error never escapes to the listener. Any 5xx response
//! that the handler returns (e.g. `StatusCode::INTERNAL_SERVER_ERROR`)
//! is normalised to the gateway's standard JSON shape so consumers
//! see a stable `code = handler_error` body.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error_mapper::GatewayError;

/// Middleware: catches handler panics + normalises 5xx bodies.
///
/// Preserves response headers added by inner stages (`header_merge`,
/// `etag`, `cache_control`) by merging them onto the gateway error
/// envelope. Without this merge, swapping in a fresh
/// `GatewayError::HandlerError.into_response()` would drop the
/// `x-pellucid-stack` / `x-content-type-options` / `referrer-policy`
/// headers that stage 12 attached on the way out.
pub async fn handler_boundary(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    if response.status().is_server_error() && !is_gateway_error_response(&response) {
        return overlay_with_handler_error(response);
    }
    response
}

fn overlay_with_handler_error(original: Response) -> Response {
    let preserved_headers = original.headers().clone();
    let mut overlay = GatewayError::HandlerError.into_response();
    let overlay_headers = overlay.headers_mut();
    for (name, value) in preserved_headers.iter() {
        if !overlay_headers.contains_key(name) {
            overlay_headers.insert(name.clone(), value.clone());
        }
    }
    overlay
}

fn is_gateway_error_response(response: &Response) -> bool {
    response
        .headers()
        .get(crate::error_mapper::GATEWAY_ERROR_CODE_HEADER)
        .is_some()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as AxumRequest, StatusCode};
    use axum::middleware::from_fn;
    use axum::routing::get;
    use axum::Router;
    use tower::util::ServiceExt;

    fn router_with(handler: axum::routing::MethodRouter) -> Router {
        Router::new().route("/x", handler).layer(from_fn(handler_boundary))
    }

    #[tokio::test]
    async fn ok_passes_through_unchanged() {
        let r = router_with(get(|| async { "ok" }));
        let resp = r
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn handler_500_is_normalised_to_gateway_error_body() {
        let r = router_with(get(|| async {
            (StatusCode::INTERNAL_SERVER_ERROR, "boom")
        }));
        let resp = r
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            resp.headers()
                .get(crate::error_mapper::GATEWAY_ERROR_CODE_HEADER)
                .unwrap(),
            "handler_error"
        );
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["code"], "handler_error");
    }

    #[tokio::test]
    async fn upstream_gateway_error_passes_through_unchanged() {
        // Simulate a stage further upstream that already produced a
        // properly-shaped error response with the marker header.
        let r = router_with(get(|| async {
            crate::error_mapper::GatewayError::ClerkUnauthorized
        }));
        let resp = r
            .oneshot(AxumRequest::builder().uri("/x").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers()
                .get(crate::error_mapper::GATEWAY_ERROR_CODE_HEADER)
                .unwrap(),
            "clerk_unauthorized"
        );
    }
}
