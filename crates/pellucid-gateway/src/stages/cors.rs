//! Stage 2 — CORS merge.
//!
//! Adds the standard `Access-Control-Allow-*` response headers based
//! on the configured [`CorsConfig`]. Runs *after* stage 1 so we know
//! the origin is in the allow-list.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{HeaderName, HeaderValue};
use axum::http::{header, Method};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::CorsConfig;

/// Per-stage state.
#[derive(Clone, Debug)]
pub struct CorsState(pub Arc<CorsConfig>);

/// Middleware: merges CORS headers onto the response.
pub async fn cors_merge(
    State(state): State<CorsState>,
    request: Request,
    next: Next,
) -> Response {
    let request_origin = request
        .headers()
        .get(header::ORIGIN)
        .cloned();
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    if let Some(origin) = request_origin {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    } else {
        // Same-origin / curl. Set `*` only when credentials are not
        // allowed; otherwise omit the header entirely so the browser
        // does not silently drop cookies.
        if !state.0.allow_credentials {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            );
        }
    }

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        join_methods(&state.0.allowed_methods),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        join_headers(&state.0.allowed_headers),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from(state.0.max_age_secs),
    );
    if state.0.allow_credentials {
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
    }
    response
}

fn join_methods(methods: &[Method]) -> HeaderValue {
    let joined = methods
        .iter()
        .map(Method::as_str)
        .collect::<Vec<_>>()
        .join(",");
    HeaderValue::try_from(joined).unwrap_or_else(|_| HeaderValue::from_static("GET,POST"))
}

fn join_headers(headers: &[HeaderName]) -> HeaderValue {
    let joined = headers
        .iter()
        .map(HeaderName::as_str)
        .collect::<Vec<_>>()
        .join(",");
    HeaderValue::try_from(joined)
        .unwrap_or_else(|_| HeaderValue::from_static("authorization,content-type"))
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

    fn router(state: CorsState) -> Router {
        Router::new()
            .route("/echo", get(|| async { "ok" }))
            .layer(from_fn_with_state(state, cors_merge))
    }

    #[tokio::test]
    async fn echoes_origin_back_when_present() {
        let cfg = CorsConfig::default();
        let resp = router(CorsState(Arc::new(cfg)))
            .oneshot(
                AxumRequest::builder()
                    .uri("/echo")
                    .header("origin", "https://app.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "https://app.test"
        );
        assert_eq!(resp.headers().get(header::VARY).unwrap(), "Origin");
        assert_eq!(
            resp.headers().get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).unwrap(),
            "true"
        );
    }

    #[tokio::test]
    async fn no_origin_with_credentials_omits_origin_header() {
        let cfg = CorsConfig::default();
        let resp = router(CorsState(Arc::new(cfg)))
            .oneshot(AxumRequest::builder().uri("/echo").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(resp
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }

    #[tokio::test]
    async fn methods_and_headers_advertised() {
        let cfg = CorsConfig::default();
        let resp = router(CorsState(Arc::new(cfg)))
            .oneshot(
                AxumRequest::builder()
                    .uri("/echo")
                    .header("origin", "https://app.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let methods = resp.headers().get(header::ACCESS_CONTROL_ALLOW_METHODS).unwrap();
        let headers = resp.headers().get(header::ACCESS_CONTROL_ALLOW_HEADERS).unwrap();
        assert!(methods.to_str().unwrap().contains("GET"));
        assert!(methods.to_str().unwrap().contains("POST"));
        assert!(methods.to_str().unwrap().contains("OPTIONS"));
        assert!(headers.to_str().unwrap().contains("authorization"));
    }

    #[tokio::test]
    async fn star_origin_only_without_credentials() {
        let cfg = CorsConfig {
            allow_credentials: false,
            ..CorsConfig::default()
        };
        let resp = router(CorsState(Arc::new(cfg)))
            .oneshot(AxumRequest::builder().uri("/echo").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            resp.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "*"
        );
    }
}
