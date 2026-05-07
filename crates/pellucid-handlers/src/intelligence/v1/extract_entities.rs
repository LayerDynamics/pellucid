//! `POST /api/intelligence/v1/extract-entities` — pellucid-ml-backed
//! named-entity extraction.
//!
//! Reads `{ text }` from the request body, calls
//! [`pellucid_ml::MlEngine::extract_entities`], returns the entity
//! list. When the binary booted without `GROQ_API_KEY` /
//! `HF_TOKEN` (so `state.ml` is `None`), responds with HTTP 503 +
//! `Retry-After: 30` so the gateway's H2-fix semantics propagate to
//! the caller.
//!
//! Tier-2 endpoint (`api_starter` plan, see
//! `crates/pellucid-auth/src/endpoint_tiers.rs` and spec §14.2). The
//! gateway middleware enforces tier before this handler runs.

use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;
use pellucid_ml::{Entity, MlError};

use crate::state::AppState;

/// Path string used by both the handler-router builder here and the
/// `ENDPOINT_ENTITLEMENTS` map in `pellucid-auth`. Keep them in
/// sync — the auth gate reads the request URI verbatim.
pub const PATH: &str = "/api/intelligence/v1/extract-entities";

/// Default `Retry-After` for 503 responses. Matches the H2 fix's
/// 30-second floor (`SPEC-001…:760`).
const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Maximum input length — 16 KiB. Larger payloads run risk of
/// inflating the Groq token bill and blowing past the 8K-token
/// model context window. Callers should chunk client-side.
const MAX_INPUT_BYTES: usize = 16 * 1024;

/// Request envelope. `text` is required and non-empty after trim.
#[derive(Debug, Deserialize)]
pub struct Request {
    pub text: String,
}

/// Response envelope. `entities` is always present; an empty list
/// means the upstream confirmed there are no entities (NOT an
/// error). `cached` is `true` when served from the in-memory /
/// SQLite cache layer rather than freshly fetched from Groq.
///
/// `Deserialize` is derived so integration tests can decode the
/// response body — the wire is always producer-side, but symmetry
/// keeps the type usable in client crates too.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractEntitiesResponse {
    pub entities: Vec<Entity>,
    /// Always `false` for now — caching is wired in a follow-up so
    /// each request hits Groq. The field is in the envelope from
    /// day one so the webview can render a "cached" badge once the
    /// cache layer ships without a breaking schema change.
    pub cached: bool,
}

/// Hand-rolled error → response conversion so the handler returns
/// the same shape for every failure mode.
///
/// 5xx responses set `x-pellucid-error: upstream_error` so the
/// gateway's `handler_boundary` middleware does NOT overlay them
/// with a generic `code = handler_error` (`pellucid-gateway/src/
/// stages/handler_boundary.rs:23-29`). Without that header the
/// gateway treats every 5xx as a panicked handler and rewrites
/// the body, dropping our 503 / 504 / 502 distinctions.
fn err_response(status: StatusCode, body: &str, retry_after: Option<u32>) -> Response {
    let body_json = serde_json::json!({
        "code": "upstream_error",
        "message": body,
    });
    let mut resp = (status, Json(body_json)).into_response();
    if let Some(sec) = retry_after {
        if let Ok(value) = HeaderValue::from_str(&sec.to_string()) {
            resp.headers_mut().insert("retry-after", value);
        }
    }
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        resp.headers_mut().insert(
            GATEWAY_ERROR_CODE_HEADER,
            HeaderValue::from_static("upstream_error"),
        );
    }
    resp
}

/// `POST /api/intelligence/v1/extract-entities` handler.
pub async fn handler(
    State(state): State<AppState>,
    Json(req): Json<Request>,
) -> Response {
    // Empty input is a 400 — don't burn an upstream call on it.
    let trimmed = req.text.trim();
    if trimmed.is_empty() {
        return err_response(StatusCode::BAD_REQUEST, "text is empty", None);
    }
    if req.text.len() > MAX_INPUT_BYTES {
        return err_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "text exceeds 16 KiB limit",
            None,
        );
    }

    // No engine wired → service unavailable. The boot path emits a
    // structured warning when this is the case so operators see why.
    let Some(engine) = state.ml.as_ref() else {
        tracing::warn!(
            target: "pellucid::handlers::intelligence",
            path = PATH,
            "MlEngine absent (GROQ_API_KEY / HF_TOKEN not set) — returning 503"
        );
        return err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "ml engine not configured",
            Some(DEFAULT_RETRY_AFTER_SECS),
        );
    };

    // 30s upstream cap: Groq's free tier averages <2s for an 8B
    // call but tail latency on a cold model can spike. We don't
    // want one slow call to wedge a request thread.
    let call = tokio::time::timeout(
        Duration::from_secs(30),
        engine.extract_entities(trimmed),
    )
    .await;

    match call {
        Ok(Ok(entities)) => {
            tracing::debug!(
                target: "pellucid::handlers::intelligence",
                count = entities.len(),
                "extract_entities ok"
            );
            (
                StatusCode::OK,
                Json(ExtractEntitiesResponse {
                    entities,
                    cached: false,
                }),
            )
                .into_response()
        }
        Ok(Err(MlError::EmptyInput(_))) => {
            err_response(StatusCode::BAD_REQUEST, "text is empty", None)
        }
        Ok(Err(MlError::MissingConfig(name))) => {
            tracing::warn!(
                target: "pellucid::handlers::intelligence",
                missing = name,
                "ml engine missing config at call time — returning 503"
            );
            err_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "ml engine not configured",
                Some(DEFAULT_RETRY_AFTER_SECS),
            )
        }
        Ok(Err(MlError::Upstream {
            endpoint,
            status,
            body,
        })) => {
            tracing::warn!(
                target: "pellucid::handlers::intelligence",
                endpoint,
                status,
                body = %body,
                "ml upstream returned non-2xx"
            );
            // 429 from Groq → propagate as 429 with Retry-After.
            // Other 4xx/5xx → 502 (bad gateway) so callers retry.
            if status == 429 {
                err_response(
                    StatusCode::TOO_MANY_REQUESTS,
                    "ml upstream rate-limited",
                    Some(DEFAULT_RETRY_AFTER_SECS),
                )
            } else {
                err_response(
                    StatusCode::BAD_GATEWAY,
                    "ml upstream error",
                    Some(DEFAULT_RETRY_AFTER_SECS),
                )
            }
        }
        Ok(Err(other)) => {
            tracing::warn!(
                target: "pellucid::handlers::intelligence",
                error = %other,
                "ml engine internal error"
            );
            err_response(StatusCode::BAD_GATEWAY, "ml engine error", Some(DEFAULT_RETRY_AFTER_SECS))
        }
        Err(_elapsed) => {
            tracing::warn!(
                target: "pellucid::handlers::intelligence",
                timeout_secs = 30,
                "ml extract_entities timed out"
            );
            err_response(
                StatusCode::GATEWAY_TIMEOUT,
                "ml upstream timed out",
                Some(DEFAULT_RETRY_AFTER_SECS),
            )
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::http::Request as HttpRequest;
    use std::sync::Arc;
    use tower::ServiceExt;

    use pellucid_ml::{MlEngine, Sentiment};

    /// Test backend that returns canned `extract_entities` responses
    /// or a chosen error.
    struct FakeMl {
        result: Result<Vec<Entity>, MlError>,
    }

    impl FakeMl {
        fn ok(entities: Vec<Entity>) -> Arc<dyn MlEngine> {
            Arc::new(Self {
                result: Ok(entities),
            })
        }
        fn err(err: MlError) -> Arc<dyn MlEngine> {
            Arc::new(Self { result: Err(err) })
        }
    }

    impl Clone for FakeMl {
        fn clone(&self) -> Self {
            // Tests construct fresh instances; clone is required by
            // the trait surface but never actually called here.
            unreachable!("FakeMl is constructed once per test")
        }
    }

    #[async_trait]
    impl MlEngine for FakeMl {
        async fn embed(&self, _: &str) -> Result<Vec<f32>, MlError> {
            Err(MlError::Unsupported("embed"))
        }
        async fn sentiment(&self, _: &str) -> Result<Sentiment, MlError> {
            Err(MlError::Unsupported("sentiment"))
        }
        async fn summarize(&self, _: &str, _: usize) -> Result<String, MlError> {
            Err(MlError::Unsupported("summarize"))
        }
        async fn extract_entities(&self, _: &str) -> Result<Vec<Entity>, MlError> {
            match &self.result {
                Ok(v) => Ok(v.clone()),
                Err(e) => match e {
                    MlError::EmptyInput(s) => Err(MlError::EmptyInput(s)),
                    MlError::MissingConfig(s) => Err(MlError::MissingConfig(s)),
                    MlError::Unsupported(s) => Err(MlError::Unsupported(s)),
                    MlError::Upstream {
                        endpoint,
                        status,
                        body,
                    } => Err(MlError::Upstream {
                        endpoint,
                        status: *status,
                        body: body.clone(),
                    }),
                    MlError::InvalidResponse { endpoint, message } => Err(MlError::InvalidResponse {
                        endpoint,
                        message: message.clone(),
                    }),
                    MlError::Decode {
                        endpoint,
                        message,
                        body,
                    } => Err(MlError::Decode {
                        endpoint,
                        message: message.clone(),
                        body: body.clone(),
                    }),
                    MlError::Http(_) => Err(MlError::Unsupported("http (untranslatable)")),
                },
            }
        }
    }

    fn router(state: AppState) -> axum::Router {
        axum::Router::new().route(PATH, axum::routing::post(handler).with_state(state))
    }

    fn post_json(body: serde_json::Value) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method("POST")
            .uri(PATH)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn returns_503_when_ml_engine_absent() {
        let state = AppState::for_tests(); // ml = None
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "Iran tested a missile."})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp.headers().get("retry-after").unwrap(),
            "30"
        );
    }

    #[tokio::test]
    async fn returns_400_for_empty_text() {
        let engine = FakeMl::ok(vec![]);
        let state = AppState::for_tests().with_ml(engine);
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "   "})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn returns_413_for_oversized_text() {
        let engine = FakeMl::ok(vec![]);
        let state = AppState::for_tests().with_ml(engine);
        let app = router(state);
        let huge = "x".repeat(MAX_INPUT_BYTES + 1);
        let resp = app
            .oneshot(post_json(serde_json::json!({ "text": huge })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn returns_200_with_entities_on_happy_path() {
        let engine = FakeMl::ok(vec![
            Entity {
                text: "Tehran".into(),
                kind: "GPE".into(),
                confidence: Some(0.9),
                start: None,
                end: None,
            },
            Entity {
                text: "IRGC".into(),
                kind: "ORG".into(),
                confidence: Some(0.85),
                start: None,
                end: None,
            },
        ]);
        let state = AppState::for_tests().with_ml(engine);
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({
                "text": "IRGC announced the missile launch from Tehran."
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 16).await.unwrap();
        let body: ExtractEntitiesResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.entities.len(), 2);
        assert_eq!(body.entities[0].text, "Tehran");
        assert_eq!(body.entities[1].kind, "ORG");
        assert!(!body.cached);
    }

    #[tokio::test]
    async fn returns_429_when_groq_rate_limits() {
        let engine = FakeMl::err(MlError::Upstream {
            endpoint: "groq.chat.extract_entities",
            status: 429,
            body: "rate limit reached".into(),
        });
        let state = AppState::for_tests().with_ml(engine);
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "Anything"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    }

    #[tokio::test]
    async fn returns_502_for_other_upstream_errors() {
        let engine = FakeMl::err(MlError::Upstream {
            endpoint: "groq.chat.extract_entities",
            status: 500,
            body: "internal".into(),
        });
        let state = AppState::for_tests().with_ml(engine);
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "Anything"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    }

    #[tokio::test]
    async fn returns_400_when_engine_emits_empty_input() {
        let engine = FakeMl::err(MlError::EmptyInput("extract_entities"));
        let state = AppState::for_tests().with_ml(engine);
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "Some text"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn returns_503_when_engine_emits_missing_config() {
        let engine = FakeMl::err(MlError::MissingConfig("GROQ_API_KEY"));
        let state = AppState::for_tests().with_ml(engine);
        let app = router(state);
        let resp = app
            .oneshot(post_json(serde_json::json!({"text": "Some text"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
