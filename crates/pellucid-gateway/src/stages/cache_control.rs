//! Stage 14 — Cache-Control.
//!
//! Sets the `Cache-Control` response header from
//! [`RouteCacheRules`]. Default is `no-store` so anonymous routes are
//! not accidentally cached at intermediaries — caller must explicitly
//! opt into a cacheable policy via `RouteCacheRules::set`.
//!
//! Runs only on successful (2xx) responses; error envelopes get
//! `Cache-Control: no-store` regardless.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{HeaderName, HeaderValue};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;

use crate::config::RouteCacheRules;

const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");
const CACHE_CONTROL_HEADER: HeaderName = header::CACHE_CONTROL;

/// Middleware: attach `Cache-Control` per route policy.
pub async fn cache_control(
    State(rules): State<Arc<RouteCacheRules>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    let policy = if response.status().is_success() {
        rules.for_path(&path).header
    } else {
        "no-store".to_string()
    };
    let value = HeaderValue::from_str(&policy).unwrap_or(NO_STORE);
    response.headers_mut().insert(CACHE_CONTROL_HEADER, value);
    response
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::config::CacheControlPolicy;
    use axum::body::Body;
    use axum::http::{Request as AxumRequest, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::Router;
    use tower::util::ServiceExt;

    fn router(rules: RouteCacheRules) -> Router {
        let rules = Arc::new(rules);
        Router::new()
            .route("/anonymous", get(|| async { "ok" }))
            .route("/cacheable", get(|| async { "fresh" }))
            .route(
                "/error",
                get(|| async { (StatusCode::FORBIDDEN, "denied") }),
            )
            .layer(from_fn_with_state(rules, cache_control))
    }

    #[tokio::test]
    async fn default_policy_is_no_store() {
        let resp = router(RouteCacheRules::new())
            .oneshot(AxumRequest::builder().uri("/anonymous").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.headers().get(CACHE_CONTROL_HEADER).unwrap(), "no-store");
    }

    #[tokio::test]
    async fn per_path_override_applied() {
        let mut rules = RouteCacheRules::new();
        rules.set("/cacheable", CacheControlPolicy::public(60));
        let resp = router(rules)
            .oneshot(AxumRequest::builder().uri("/cacheable").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            resp.headers().get(CACHE_CONTROL_HEADER).unwrap(),
            "public, max-age=60"
        );
    }

    #[tokio::test]
    async fn error_responses_get_no_store_regardless_of_rules() {
        let mut rules = RouteCacheRules::new();
        rules.set("/error", CacheControlPolicy::public(3600));
        let resp = router(rules)
            .oneshot(AxumRequest::builder().uri("/error").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(resp.headers().get(CACHE_CONTROL_HEADER).unwrap(), "no-store");
    }
}
