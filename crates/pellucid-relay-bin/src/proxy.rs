//! Proxy routes — `/opensky/*`.
//!
//! The relay's job is to consolidate upstream credentials in
//! one place: the `/opensky/states/all` proxy lets the public
//! API binary (and seeders) call OpenSky without each crate
//! holding its own OAuth2 token. The proxy is gated by the
//! relay shared-secret middleware (auth.rs).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;
use std::sync::Arc;

use pellucid_streams::{OpenSkyClient, StreamsError};

/// HTTP path the proxy mounts under.
pub const OPENSKY_PROXY_PREFIX: &str = "/opensky";

/// Proxy state shared across handlers.
#[derive(Clone, Debug)]
pub struct ProxyState {
    /// OpenSky upstream client. `Arc` for cheap clones across
    /// per-request handlers.
    pub opensky: Arc<OpenSkyClient>,
}

/// Mount the `/opensky/*` routes.
pub fn opensky_router(state: ProxyState) -> Router {
    Router::new()
        .route("/opensky/states/path/{path}", get(states_for_path))
        .route("/opensky/states/all", get(states_all))
        .with_state(state)
}

/// `GET /opensky/states/all` — full live state-vector list.
async fn states_all(State(state): State<ProxyState>) -> impl IntoResponse {
    match state.opensky.fetch_path("/states/all").await {
        Ok(Some(body)) => {
            ::metrics::counter!("pellucid_proxy_opensky_ok").increment(1);
            (StatusCode::OK, Json(body_to_json(body))).into_response()
        }
        Ok(None) => {
            ::metrics::counter!("pellucid_proxy_opensky_negative").increment(1);
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "no upstream data"})),
            )
                .into_response()
        }
        Err(e) => streams_error_response(&e),
    }
}

/// `GET /opensky/states/path/{path}` — pass-through for
/// arbitrary upstream paths. The `path` segment is a
/// URL-encoded path string starting with `/states/`.
async fn states_for_path(
    State(state): State<ProxyState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    let decoded = match urlencoding_decode(&path) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("path decode: {e}")})),
            )
                .into_response();
        }
    };
    if !decoded.starts_with("/states/") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path must start with /states/"})),
        )
            .into_response();
    }
    match state.opensky.fetch_path(&decoded).await {
        Ok(Some(body)) => {
            ::metrics::counter!("pellucid_proxy_opensky_ok").increment(1);
            (StatusCode::OK, Json(body_to_json(body))).into_response()
        }
        Ok(None) => {
            ::metrics::counter!("pellucid_proxy_opensky_negative").increment(1);
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "no upstream data"})),
            )
                .into_response()
        }
        Err(e) => streams_error_response(&e),
    }
}

fn streams_error_response(err: &StreamsError) -> axum::response::Response {
    ::metrics::counter!("pellucid_proxy_opensky_error").increment(1);
    let (code, msg) = match err {
        StreamsError::Status { status } if *status >= 500 => {
            (StatusCode::BAD_GATEWAY, format!("upstream status {status}"))
        }
        StreamsError::Status { status } => (
            StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY),
            format!("upstream status {status}"),
        ),
        StreamsError::Io(s) => (StatusCode::BAD_GATEWAY, format!("upstream io: {s}")),
        StreamsError::Parse(s) => (StatusCode::BAD_GATEWAY, format!("upstream parse: {s}")),
        StreamsError::NotFound => (StatusCode::NOT_FOUND, "upstream not found".to_string()),
    };
    (code, Json(serde_json::json!({"error": msg}))).into_response()
}

fn body_to_json(body: pellucid_streams::OpenSkyResponse) -> Value {
    serde_json::to_value(&body).unwrap_or(Value::Null)
}

fn urlencoding_decode(s: &str) -> Result<String, String> {
    // RFC 3986 percent-decoding limited to printable ASCII.
    // We don't need full UTF-8 decoding for OpenSky paths.
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'%' {
            if i + 2 >= bytes.len() {
                return Err(format!("truncated percent-encoded byte at offset {i}"));
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .map_err(|e| format!("non-ascii hex at {i}: {e}"))?;
            let byte = u8::from_str_radix(hex, 16)
                .map_err(|e| format!("invalid hex {hex:?} at {i}: {e}"))?;
            out.push(byte as char);
            i += 3;
        } else {
            out.push(c as char);
            i += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn urlencoding_decode_handles_percent_encoded_slash() {
        assert_eq!(
            urlencoding_decode("%2Fstates%2Fall").unwrap(),
            "/states/all"
        );
    }

    #[test]
    fn urlencoding_decode_round_trips_plain_ascii() {
        assert_eq!(urlencoding_decode("/states/all").unwrap(), "/states/all");
    }

    #[test]
    fn urlencoding_decode_truncated_percent_yields_error() {
        assert!(urlencoding_decode("/states%").is_err());
        assert!(urlencoding_decode("/states%2").is_err());
    }

    #[test]
    fn urlencoding_decode_invalid_hex_yields_error() {
        assert!(urlencoding_decode("/states%ZZ").is_err());
    }
}
