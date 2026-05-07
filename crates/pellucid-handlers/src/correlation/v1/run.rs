//! `POST /api/correlation/v1/run` — cross-domain convergence cards.
//!
//! Accepts a request body of pre-assembled `SignalEvidence` per
//! domain (`military` / `escalation` / `economic` / `disaster`),
//! runs them through `pellucid_correlation::CorrelationEngine`
//! against the matching adapter, and returns the resulting
//! `ConvergenceCard` rows for each domain — sorted by score
//! descending and filtered to the adapter's threshold.
//!
//! The handler does NOT collect signals from cached envelopes
//! itself — the JS source's `collectSignals` reads from a webview
//! `AppContext.intelligenceCache` shape that doesn't translate
//! 1:1 to the server cache. Web SaaS customers POST signals they
//! already have; the desktop sidecar wires its own collector that
//! reads from the local SQLite.
//!
//! Tier-2 endpoint
//! (`pellucid-auth::ML_ENDPOINT_ENTITLEMENTS:/api/correlation/v1/run`).

use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use pellucid_correlation::{
    ConvergenceCard, CorrelationDomain, CorrelationEngine, DisasterAdapter, EconomicAdapter,
    EscalationAdapter, MilitaryAdapter, SignalEvidence,
};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Endpoint path — mirrored in
/// `pellucid-auth::ML_ENDPOINT_ENTITLEMENTS`.
pub const PATH: &str = "/api/correlation/v1/run";

/// Default `Retry-After` for 503 responses (H2 fix semantics).
const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Max signals per domain — protects against accidental DoS where a
/// caller dumps tens of thousands of points and the engine spends
/// the whole request in grid-union-find. 5,000 per domain × 4
/// domains = 20K total worst-case, comfortably under a 30 s wall
/// budget.
const MAX_SIGNALS_PER_DOMAIN: usize = 5_000;

#[derive(Debug, Default, Deserialize)]
pub struct Request {
    /// Signals for the military adapter. Empty / missing → that
    /// domain returns `cards: []` in the response.
    #[serde(default)]
    pub military: Vec<SignalEvidence>,
    #[serde(default)]
    pub escalation: Vec<SignalEvidence>,
    #[serde(default)]
    pub economic: Vec<SignalEvidence>,
    #[serde(default)]
    pub disaster: Vec<SignalEvidence>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DomainCards {
    pub domain: CorrelationDomain,
    pub cards: Vec<ConvergenceCard>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CorrelationRunResponse {
    pub domains: Vec<DomainCards>,
    /// UNIX millisecond epoch the engine ran at — useful for the
    /// webview to drive its "last updated" indicator.
    #[serde(rename = "computedAtMs")]
    pub computed_at_ms: i64,
}

/// Same convention as the intelligence handlers: 5xx + 429 carry
/// `x-pellucid-error: upstream_error` so the gateway boundary
/// doesn't overlay them with a generic `handler_error` envelope.
fn err_response(status: StatusCode, body: &str, retry_after: Option<u32>) -> Response {
    let body_json = serde_json::json!({
        "code": "upstream_error",
        "message": body,
    });
    let mut resp = (status, Json(body_json)).into_response();
    if let Some(secs) = retry_after {
        if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
            resp.headers_mut().insert("retry-after", v);
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

fn validate(req: &Request) -> Result<(), Box<Response>> {
    let total = req.military.len() + req.escalation.len() + req.economic.len() + req.disaster.len();
    if total == 0 {
        return Err(Box::new(err_response(
            StatusCode::BAD_REQUEST,
            "no signals supplied for any domain",
            None,
        )));
    }
    for (name, len) in [
        ("military", req.military.len()),
        ("escalation", req.escalation.len()),
        ("economic", req.economic.len()),
        ("disaster", req.disaster.len()),
    ] {
        if len > MAX_SIGNALS_PER_DOMAIN {
            return Err(Box::new(err_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                &format!("{name} signals ({len}) exceed per-domain cap {MAX_SIGNALS_PER_DOMAIN}"),
                None,
            )));
        }
    }
    Ok(())
}

pub async fn handler(State(_state): State<AppState>, Json(req): Json<Request>) -> Response {
    if let Err(r) = validate(&req) {
        return *r;
    }

    let now = chrono::Utc::now();
    // 30 s wall budget — proximity union-find at 5K points per
    // domain × 4 is well under a second on modern hardware, but a
    // catastrophic input pattern (every signal in one cell) could
    // blow this. The timeout caps that.
    let computation = tokio::task::spawn_blocking(move || {
        let mut engine = CorrelationEngine::new();
        let military_adapter = MilitaryAdapter::new();
        let escalation_adapter = EscalationAdapter::new();
        let economic_adapter = EconomicAdapter::new();
        let disaster_adapter = DisasterAdapter::new();

        let military_cards = engine.run(&military_adapter, req.military, now);
        let escalation_cards = engine.run(&escalation_adapter, req.escalation, now);
        let economic_cards = engine.run(&economic_adapter, req.economic, now);
        let disaster_cards = engine.run(&disaster_adapter, req.disaster, now);

        vec![
            DomainCards {
                domain: CorrelationDomain::Military,
                cards: military_cards,
            },
            DomainCards {
                domain: CorrelationDomain::Escalation,
                cards: escalation_cards,
            },
            DomainCards {
                domain: CorrelationDomain::Economic,
                cards: economic_cards,
            },
            DomainCards {
                domain: CorrelationDomain::Disaster,
                cards: disaster_cards,
            },
        ]
    });

    let domains = match tokio::time::timeout(Duration::from_secs(30), computation).await {
        Ok(Ok(v)) => v,
        Ok(Err(join_err)) => {
            tracing::error!(target: "pellucid::handlers::correlation", error = %join_err, "spawn_blocking panicked");
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "correlation engine panicked",
                Some(DEFAULT_RETRY_AFTER_SECS),
            );
        }
        Err(_elapsed) => {
            tracing::warn!(target: "pellucid::handlers::correlation", timeout_secs = 30, "correlation run timed out");
            return err_response(
                StatusCode::GATEWAY_TIMEOUT,
                "correlation run timed out",
                Some(DEFAULT_RETRY_AFTER_SECS),
            );
        }
    };

    (
        StatusCode::OK,
        Json(CorrelationRunResponse {
            domains,
            computed_at_ms: now.timestamp_millis(),
        }),
    )
        .into_response()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request as HttpRequest;
    use serde_json::json;
    use tower::ServiceExt;

    fn router(state: AppState) -> axum::Router {
        axum::Router::new().route(PATH, axum::routing::post(handler).with_state(state))
    }

    fn post(body: serde_json::Value) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method("POST")
            .uri(PATH)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn returns_400_when_no_signals_at_all() {
        let app = router(AppState::for_tests());
        let resp = app.oneshot(post(json!({}))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn returns_413_when_a_domain_exceeds_cap() {
        let app = router(AppState::for_tests());
        let oversize: Vec<serde_json::Value> = (0..MAX_SIGNALS_PER_DOMAIN + 1)
            .map(|_| {
                json!({
                    "type": "conflict_event",
                    "source": "t",
                    "severity": 50,
                    "country": "UA",
                    "timestamp": 0,
                    "label": "x"
                })
            })
            .collect();
        let resp = app
            .oneshot(post(json!({ "escalation": oversize })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn returns_200_with_empty_domain_cards_for_unmatched_signals() {
        let app = router(AppState::for_tests());
        // Single signal — below the cluster size (need ≥ 2) so no
        // cards emitted, but the request is well-formed and
        // non-empty so we still get a 200.
        let resp = app
            .oneshot(post(json!({
                "escalation": [{
                    "type": "conflict_event",
                    "source": "t",
                    "severity": 50,
                    "country": "UA",
                    "timestamp": 0,
                    "label": "x"
                }]
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: CorrelationRunResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.domains.len(), 4);
        for d in &body.domains {
            assert!(d.cards.is_empty(), "expected empty cards on {:?}", d.domain);
        }
        assert!(body.computed_at_ms > 0);
    }

    #[tokio::test]
    async fn returns_cards_for_two_member_country_cluster() {
        // Country mode (escalation) with two `conflict_event`
        // signals → cluster size 2 → score above threshold → card.
        let app = router(AppState::for_tests());
        let resp = app
            .oneshot(post(json!({
                "escalation": [
                    {
                        "type": "conflict_event",
                        "source": "acled",
                        "severity": 70,
                        "country": "UA",
                        "timestamp": 0,
                        "label": "Kyiv strike"
                    },
                    {
                        "type": "conflict_event",
                        "source": "acled",
                        "severity": 60,
                        "country": "UA",
                        "timestamp": 0,
                        "label": "Kharkiv strike"
                    }
                ]
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: CorrelationRunResponse = serde_json::from_slice(&bytes).unwrap();
        let escalation = body
            .domains
            .iter()
            .find(|d| d.domain == CorrelationDomain::Escalation)
            .unwrap();
        assert_eq!(escalation.cards.len(), 1);
        assert!(escalation.cards[0].countries.contains(&"UA".to_string()));
        assert!(escalation.cards[0].title.contains("UA"));
    }

    #[tokio::test]
    async fn each_request_runs_with_fresh_engine_state() {
        // Two separate requests with the same signals → both must
        // emit `Stable` trend (no carry-over from one request to
        // the next, because the handler builds a fresh engine per
        // call). Pinning this prevents accidental statefulness if
        // someone later moves the engine into AppState.
        let app = router(AppState::for_tests());
        for _ in 0..2 {
            let resp = app
                .clone()
                .oneshot(post(json!({
                    "escalation": [
                        {
                            "type": "conflict_event",
                            "source": "acled",
                            "severity": 70,
                            "country": "UA",
                            "timestamp": 0,
                            "label": "x"
                        },
                        {
                            "type": "conflict_event",
                            "source": "acled",
                            "severity": 70,
                            "country": "UA",
                            "timestamp": 0,
                            "label": "y"
                        }
                    ]
                })))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
            let body: CorrelationRunResponse = serde_json::from_slice(&bytes).unwrap();
            let card = body
                .domains
                .iter()
                .find(|d| d.domain == CorrelationDomain::Escalation)
                .unwrap()
                .cards
                .first()
                .unwrap();
            assert_eq!(card.trend, pellucid_correlation::TrendDirection::Stable);
        }
    }
}
