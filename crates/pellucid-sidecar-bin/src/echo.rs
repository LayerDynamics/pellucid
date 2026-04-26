//! `/api/echo` route handler — the only T1.9 endpoint.
//!
//! Accepts either a JSON body (POST) or no body (GET) and returns a
//! payload describing what the sidecar saw. Used by the desktop e2e
//! suite as a heartbeat that confirms (a) the sidecar is reachable on
//! its dynamic port, (b) the bearer middleware is wired correctly, and
//! (c) the webview's `toApiUrl` plus fetch patch produces a working
//! URL.

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

/// Query string accepted by the GET form of `/api/echo`. Optional;
/// missing fields fall back to `None`.
#[derive(Debug, Deserialize)]
pub struct EchoQuery {
    /// Free-form text the sidecar should echo back.
    pub message: Option<String>,
}

/// Body accepted by the POST form of `/api/echo`.
#[derive(Debug, Deserialize)]
pub struct EchoBody {
    /// Free-form text the sidecar should echo back.
    pub message: Option<String>,
    /// Arbitrary nested JSON the sidecar should echo back unchanged.
    pub payload: Option<serde_json::Value>,
}

/// Response shape returned by `/api/echo`. Stable across GET/POST so
/// the webview's e2e spec can assert against it uniformly.
#[derive(Debug, Serialize, Deserialize)]
pub struct EchoResponse {
    /// The text the caller passed in (or `null`).
    pub message: Option<String>,
    /// Arbitrary nested JSON the caller passed in (or `null`).
    pub payload: Option<serde_json::Value>,
    /// Sidecar uptime in milliseconds at the moment the request hit.
    pub uptime_ms: u64,
}

pub(crate) async fn echo_get(
    Query(q): Query<EchoQuery>,
    axum::extract::State(state): axum::extract::State<crate::server::SharedAppState>,
) -> impl IntoResponse {
    let body = EchoResponse {
        message: q.message,
        payload: None,
        uptime_ms: state.uptime_ms(),
    };
    (StatusCode::OK, Json(body))
}

pub(crate) async fn echo_post(
    axum::extract::State(state): axum::extract::State<crate::server::SharedAppState>,
    body: Option<Json<EchoBody>>,
) -> impl IntoResponse {
    let resp = if let Some(Json(b)) = body {
        EchoResponse {
            message: b.message,
            payload: b.payload,
            uptime_ms: state.uptime_ms(),
        }
    } else {
        EchoResponse {
            message: None,
            payload: None,
            uptime_ms: state.uptime_ms(),
        }
    };
    (StatusCode::OK, Json(resp))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn echo_response_serialises_with_null_for_missing_fields() {
        let resp = EchoResponse {
            message: None,
            payload: None,
            uptime_ms: 1234,
        };
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"message\":null"));
        assert!(s.contains("\"payload\":null"));
        assert!(s.contains("\"uptime_ms\":1234"));
    }

    #[test]
    fn echo_response_round_trips_via_serde() {
        let original = EchoResponse {
            message: Some("ping".into()),
            payload: Some(serde_json::json!({"k": 1})),
            uptime_ms: 9000,
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: EchoResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.message.as_deref(), Some("ping"));
        assert_eq!(back.payload, Some(serde_json::json!({"k": 1})));
        assert_eq!(back.uptime_ms, 9000);
    }
}
