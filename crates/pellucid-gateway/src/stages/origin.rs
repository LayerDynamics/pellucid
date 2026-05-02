//! Stage 1 — Origin allow-list.
//!
//! Returns 403 + `code = origin_forbidden` when an inbound request
//! carries an `Origin` header that is not in the configured
//! [`OriginAllowList`]. Empty `Origin` (curl, same-origin GETs) is
//! always allowed; the browser CORS layer is the actual cross-origin
//! boundary.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::config::OriginAllowList;
use crate::error_mapper::GatewayError;

/// Middleware function consumed by `from_fn_with_state`. The state
/// is the shared [`OriginAllowList`] itself — no wrapper newtype.
pub async fn origin_allow_list(
    State(allow): State<Arc<OriginAllowList>>,
    request: Request,
    next: Next,
) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !allow.permits(origin) {
        return GatewayError::OriginForbidden.into_response();
    }
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
    use axum::Router;
    use tower::util::ServiceExt;

    fn router(allow: Arc<OriginAllowList>) -> Router {
        Router::new()
            .route("/echo", get(|| async { "ok" }))
            .layer(from_fn_with_state(allow, origin_allow_list))
    }

    fn req(origin: Option<&str>) -> AxumRequest<Body> {
        let mut b = AxumRequest::builder().uri("/echo").method("GET");
        if let Some(o) = origin {
            b = b.header("origin", o);
        }
        b.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn allows_empty_origin() {
        let allow = Arc::new(OriginAllowList::new());
        let resp = router(allow).oneshot(req(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn forbids_unknown_origin() {
        let allow = Arc::new(OriginAllowList::new());
        let resp = router(allow)
            .oneshot(req(Some("https://evil.example")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn allows_exact_origin() {
        let mut allow = OriginAllowList::new();
        allow.allow_exact("https://worldmonitor.app");
        let resp = router(Arc::new(allow))
            .oneshot(req(Some("https://worldmonitor.app")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn allows_loopback_when_flag_set() {
        let allow = OriginAllowList::new().with_loopback(true);
        let resp = router(Arc::new(allow))
            .oneshot(req(Some("http://127.0.0.1:5173")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
