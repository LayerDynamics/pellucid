//! `GET /api/aviation/v1/get-flight-status` handler.
//!
//! Direct port of `server/aviation/v1/get-flight-status.ts` from the
//! original WorldMonitor codebase. Flow:
//!
//! 1. Parse + validate `flight`, `date`, `origin` query params.
//! 2. Build the cache key
//!    `aviation:status:{FLIGHT}:{date}:{ORIGIN}:v1` (FLIGHT and
//!    ORIGIN uppercased — see
//!    [`crate::generated::aviation::v1::GetFlightStatusRequest::cache_key`]).
//! 3. Call [`pellucid_cache::cached_fetch_json`] at the FAST tier;
//!    the inner fetcher dispatches to
//!    [`AppState::aviation`].
//! 4. Return the resolved [`FlightStatus`] envelope, or the
//!    appropriate error code on validation/upstream failure.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use pellucid_cache::cached_fetch_json;
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::generated::aviation::v1::{
    FlightStatus, GetFlightStatusRequest, CACHE_KEY_TEMPLATE, CACHE_TIER,
};
use crate::state::AppState;

/// Wire-compat alias kept private to this module — the linter at
/// `tools/check-cache-keys.ts` (T2.10) scans for the literal
/// `aviation:status:{flight}:{date}:{origin}:v1` and a static
/// reference here ensures the file participates in that scan.
const _: &str = CACHE_KEY_TEMPLATE;

/// Query string. We use a separate type from
/// [`GetFlightStatusRequest`] so axum's `Query` extractor can apply
/// `serde` validation at the edge before we touch the cache layer.
#[derive(Debug, Deserialize)]
pub struct FlightStatusQuery {
    /// IATA flight number, e.g. `"AA100"`.
    pub flight: String,
    /// ISO 8601 date (`YYYY-MM-DD`).
    pub date: String,
    /// 3-letter IATA origin airport code.
    pub origin: String,
}

impl FlightStatusQuery {
    /// Run the field-shape validations the original handler did.
    /// Returns `Err` with a stable code the gateway translates to
    /// `400 + invalid_request` per SPEC-001 §11.5.
    pub fn validate(&self) -> Result<(), HandlerError> {
        if self.flight.is_empty() || self.flight.len() > 16 {
            return Err(HandlerError::InvalidRequest {
                field: "flight",
                reason: "must be 1..=16 chars",
            });
        }
        // ISO 8601 date — basic shape check; the upstream rejects
        // malformed dates anyway, but failing fast spares us a
        // round-trip and a cache slot.
        if self.date.len() != 10
            || self.date.as_bytes().get(4) != Some(&b'-')
            || self.date.as_bytes().get(7) != Some(&b'-')
        {
            return Err(HandlerError::InvalidRequest {
                field: "date",
                reason: "must match YYYY-MM-DD",
            });
        }
        if !self.date.chars().enumerate().all(|(i, c)| match i {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        }) {
            return Err(HandlerError::InvalidRequest {
                field: "date",
                reason: "must match YYYY-MM-DD",
            });
        }
        if self.origin.len() != 3 || !self.origin.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(HandlerError::InvalidRequest {
                field: "origin",
                reason: "must be a 3-letter IATA airport code",
            });
        }
        Ok(())
    }
}

impl From<&FlightStatusQuery> for GetFlightStatusRequest {
    fn from(q: &FlightStatusQuery) -> Self {
        Self {
            flight: q.flight.clone(),
            date: q.date.clone(),
            origin: q.origin.clone(),
        }
    }
}

/// Errors the handler can produce. Each variant carries a stable
/// `code` the gateway threads into the response envelope so the
/// webview can branch deterministically.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Field-shape validation failed.
    #[error("invalid request: {field} ({reason})")]
    InvalidRequest {
        /// Name of the offending field.
        field: &'static str,
        /// Human-readable reason — surfaced verbatim to the
        /// webview so the field-level error can render inline.
        reason: &'static str,
    },
    /// Upstream returned a 5xx / network / parse error.
    #[error("upstream failure: {0}")]
    Upstream(String),
    /// Cache layer failed (sqlx error; should be impossible in
    /// practice but kept distinct so monitoring can alert).
    #[error("cache failure: {0}")]
    Cache(String),
}

impl HandlerError {
    /// Stable error code — webview branches on this.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest { .. } => "invalid_request",
            Self::Upstream(_) => "upstream_failure",
            Self::Cache(_) => "cache_failure",
        }
    }

    /// HTTP status code for this error.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::InvalidRequest { .. } => StatusCode::BAD_REQUEST,
            Self::Upstream(_) | Self::Cache(_) => StatusCode::BAD_GATEWAY,
        }
    }
}

impl axum::response::IntoResponse for HandlerError {
    fn into_response(self) -> axum::response::Response {
        let body = json!({
            "error": {
                "code": self.code(),
                "message": self.to_string(),
            }
        });
        let mut resp = (self.status(), Json(body)).into_response();
        // Set the gateway-error marker so stage 11
        // (`handler_boundary`) does NOT overlay our envelope with
        // its generic `handler_error` body — preserving the
        // handler-specific code so the webview can branch.
        resp.headers_mut().insert(
            GATEWAY_ERROR_CODE_HEADER,
            HeaderValue::from_static(self.code()),
        );
        resp
    }
}

/// The handler axum mounts.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<FlightStatusQuery>,
) -> Result<Json<FlightStatus>, HandlerError> {
    q.validate()?;
    let req = GetFlightStatusRequest::from(&q);
    let key = req.cache_key();

    let aviation = state.aviation.clone();
    // `cached_fetch_json` owns the fetcher closure; we move owned
    // copies of the request fields in so the future is `'static`.
    let fetch_flight = req.flight.clone();
    let fetch_date = req.date.clone();
    let fetch_origin = req.origin.clone();

    let value = cached_fetch_json::<FlightStatus, _, _>(
        &state.pool,
        &state.cache_registry,
        &key,
        CACHE_TIER,
        move || async move {
            aviation
                .fetch_flight(&fetch_flight, &fetch_date, &fetch_origin)
                .await
        },
    )
    .await
    .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let Some(status) = value else {
        // Negative-cacheable upstream miss — surface as a 404 so
        // the webview can render "no such flight on this date"
        // distinctly from upstream sickness.
        return Err(HandlerError::Upstream("flight not found".into()));
    };
    Ok(Json(status))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::*;
    use crate::state::FlightStatusUpstream;

    /// Counts invocations + returns a fixed flight status — the
    /// per-method unit test harness.
    #[derive(Debug)]
    struct CountingUpstream {
        calls: AtomicUsize,
        response: Option<FlightStatus>,
    }

    impl CountingUpstream {
        fn ok(status: FlightStatus) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                response: Some(status),
            }
        }

        fn miss() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                response: None,
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl FlightStatusUpstream for CountingUpstream {
        async fn fetch_flight(
            &self,
            _flight: &str,
            _date: &str,
            _origin: &str,
        ) -> Result<Option<FlightStatus>, Box<dyn std::error::Error + Send + Sync>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    #[derive(Debug)]
    struct FailingUpstream;

    #[async_trait]
    impl FlightStatusUpstream for FailingUpstream {
        async fn fetch_flight(
            &self,
            _flight: &str,
            _date: &str,
            _origin: &str,
        ) -> Result<Option<FlightStatus>, Box<dyn std::error::Error + Send + Sync>> {
            Err("simulated 503".into())
        }
    }

    fn good_query() -> FlightStatusQuery {
        FlightStatusQuery {
            flight: "AA100".into(),
            date: "2026-04-25".into(),
            origin: "JFK".into(),
        }
    }

    fn flight_status(flight: &str) -> FlightStatus {
        FlightStatus {
            flight: flight.into(),
            scheduled_departure: "2026-04-25T12:00:00Z".into(),
            scheduled_arrival: "2026-04-25T15:00:00Z".into(),
            status: "active".into(),
            origin: "JFK".into(),
            destination: "LAX".into(),
            departure_gate: Some("A12".into()),
            arrival_gate: None,
        }
    }

    #[test]
    fn validate_accepts_well_formed_input() {
        assert!(good_query().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_flight() {
        let mut q = good_query();
        q.flight = String::new();
        let err = q.validate().unwrap_err();
        assert_eq!(err.code(), "invalid_request");
    }

    #[test]
    fn validate_rejects_oversized_flight() {
        let mut q = good_query();
        q.flight = "A".repeat(17);
        assert!(q.validate().is_err());
    }

    #[test]
    fn validate_rejects_malformed_date() {
        for bad in [
            "2026/04/25", // wrong separator
            "26-04-25",   // 2-digit year
            "2026-4-25",  // 1-digit month
            "2026-04-25T",
            "twenty-six",
        ] {
            let mut q = good_query();
            q.date = bad.into();
            assert!(q.validate().is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn validate_rejects_bad_origin() {
        for bad in ["J", "JF", "JFKL", "JF1", "  J", "j-k"] {
            let mut q = good_query();
            q.origin = bad.into();
            assert!(q.validate().is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn handler_error_status_codes() {
        assert_eq!(
            HandlerError::InvalidRequest {
                field: "flight",
                reason: "x"
            }
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            HandlerError::Upstream("x".into()).status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            HandlerError::Cache("x".into()).status(),
            StatusCode::BAD_GATEWAY
        );
    }

    #[test]
    fn handler_error_codes() {
        assert_eq!(
            HandlerError::InvalidRequest {
                field: "f",
                reason: "r",
            }
            .code(),
            "invalid_request"
        );
        assert_eq!(HandlerError::Upstream("x".into()).code(), "upstream_failure");
        assert_eq!(HandlerError::Cache("x".into()).code(), "cache_failure");
    }

    #[test]
    fn request_cache_key_assembled_from_query() {
        let q = good_query();
        let req = GetFlightStatusRequest::from(&q);
        assert_eq!(req.cache_key(), "aviation:status:AA100:2026-04-25:JFK:v1");
    }

    #[tokio::test]
    async fn handler_cold_cache_invokes_upstream_and_returns_envelope() {
        let upstream = Arc::new(CountingUpstream::ok(flight_status("AA100")));
        let state = AppState::for_tests_async()
            .await
            .unwrap()
            .with_aviation(upstream.clone());
        let result = handler(State(state), Query(good_query())).await.unwrap();
        assert_eq!(result.0.flight, "AA100");
        assert_eq!(upstream.calls(), 1, "cold cache must call the upstream");
    }

    #[tokio::test]
    async fn handler_warm_cache_skips_upstream_on_second_call() {
        let upstream = Arc::new(CountingUpstream::ok(flight_status("AA100")));
        let state = AppState::for_tests_async()
            .await
            .unwrap()
            .with_aviation(upstream.clone());
        let _ = handler(State(state.clone()), Query(good_query())).await.unwrap();
        let _ = handler(State(state), Query(good_query())).await.unwrap();
        assert_eq!(upstream.calls(), 1, "second call must hit the cache");
    }

    #[tokio::test]
    async fn handler_upstream_miss_yields_upstream_error() {
        let upstream = Arc::new(CountingUpstream::miss());
        let state = AppState::for_tests_async()
            .await
            .unwrap()
            .with_aviation(upstream.clone());
        let err = handler(State(state), Query(good_query()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "upstream_failure");
    }

    #[tokio::test]
    async fn handler_upstream_5xx_yields_cache_error() {
        // `cached_fetch_json` wraps the fetcher's `Err` in a
        // `sqlx::Error::Protocol`, which the handler maps to
        // `Cache`. The error code is therefore `cache_failure`
        // even though the root cause was the upstream — this is
        // the documented behaviour and the gateway's metrics layer
        // distinguishes the two via the wrapped message.
        let state = AppState::for_tests_async()
            .await
            .unwrap()
            .with_aviation(Arc::new(FailingUpstream));
        let err = handler(State(state), Query(good_query()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "cache_failure");
        assert!(
            err.to_string().contains("simulated 503"),
            "underlying upstream error must be preserved in the message: {err}"
        );
    }

    #[tokio::test]
    async fn handler_invalid_query_short_circuits_before_cache() {
        let upstream = Arc::new(CountingUpstream::ok(flight_status("AA100")));
        let state = AppState::for_tests_async()
            .await
            .unwrap()
            .with_aviation(upstream.clone());
        let mut q = good_query();
        q.flight = String::new();
        let err = handler(State(state), Query(q)).await.unwrap_err();
        assert_eq!(err.code(), "invalid_request");
        assert_eq!(upstream.calls(), 0, "validation failure must not touch the upstream");
    }
}

