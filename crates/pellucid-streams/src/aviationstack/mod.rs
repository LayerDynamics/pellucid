//! aviationstack.com REST v1 client — flights endpoint.
//!
//! Endpoint shape (production):
//! ```text
//! GET https://api.aviationstack.com/v1/flights
//!     ?access_key=...
//!     &flight_iata=AA100
//!     &flight_date=2026-04-25
//!     &dep_iata=JFK
//! ```
//!
//! Response: `{ data: [ { flight_status, departure: {...},
//! arrival: {...}, flight: { iata } } ], pagination: {...} }`. We
//! pluck the first `data[]` element when present, map it onto the
//! sebuf-generated [`FlightStatus`] struct, and let the cache layer
//! handle envelope wrapping.
//!
//! The [`AviationstackClient`] takes an injected `reqwest::Client`
//! and base URL so integration tests can point it at a `wiremock`
//! server.

use serde::Deserialize;
use url::Url;

use pellucid_handlers::generated::aviation::v1::FlightStatus;

use crate::error::StreamsError;

/// Configuration for the aviationstack client.
#[derive(Clone, Debug)]
pub struct AviationstackConfig {
    /// Base URL ending in `/v1` (no trailing slash). Default
    /// `https://api.aviationstack.com/v1`.
    pub base_url: String,
    /// `access_key` query parameter — the per-tenant API token.
    pub access_key: String,
}

impl AviationstackConfig {
    /// Build a config with the production base URL and the supplied
    /// access key.
    #[must_use]
    pub fn with_access_key(access_key: impl Into<String>) -> Self {
        Self {
            base_url: "https://api.aviationstack.com/v1".to_string(),
            access_key: access_key.into(),
        }
    }
}

/// Pluggable aviationstack client.
#[derive(Clone, Debug)]
pub struct AviationstackClient {
    http: reqwest::Client,
    config: AviationstackConfig,
}

impl AviationstackClient {
    /// Build a client against `config` with the given HTTP client.
    #[must_use]
    pub fn new(config: AviationstackConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Fetch a single flight by IATA + date + origin. Returns
    /// `Ok(None)` when the upstream's `data[]` is empty (a legitimate
    /// "no such flight on this date") so the cache can record a
    /// negative sentinel.
    ///
    /// # Errors
    ///
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for upstream non-2xx.
    /// - [`StreamsError::Parse`] when the body shape is unexpected.
    pub async fn fetch_flight(
        &self,
        flight: &str,
        date: &str,
        origin: &str,
    ) -> Result<Option<FlightStatus>, StreamsError> {
        let mut url = Url::parse(&format!("{}/flights", self.config.base_url))
            .map_err(|e| StreamsError::Parse(format!("base url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("access_key", &self.config.access_key);
            q.append_pair("flight_iata", flight);
            q.append_pair("flight_date", date);
            q.append_pair("dep_iata", origin);
        }

        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }

        let body: AviationstackResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;

        let Some(first) = body.data.into_iter().next() else {
            return Ok(None);
        };

        Ok(Some(map_to_flight_status(first, flight, origin)))
    }
}

/// Aviationstack `flights` response envelope. We only deserialise
/// the fields we actually consume; the upstream emits more
/// (pagination, airline/aircraft metadata, geo coordinates) but
/// they are not needed by the FAST-tier flight-status panel.
#[derive(Debug, Deserialize)]
struct AviationstackResponse {
    #[serde(default)]
    data: Vec<AviationstackFlight>,
}

#[derive(Debug, Default, Deserialize)]
struct AviationstackFlight {
    #[serde(default)]
    flight_status: String,
    #[serde(default)]
    departure: AviationstackEndpoint,
    #[serde(default)]
    arrival: AviationstackEndpoint,
    #[serde(default)]
    flight: AviationstackFlightId,
}

#[derive(Debug, Default, Deserialize)]
struct AviationstackEndpoint {
    #[serde(default)]
    iata: String,
    #[serde(default)]
    scheduled: String,
    #[serde(default)]
    gate: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct AviationstackFlightId {
    #[serde(default)]
    iata: String,
}

fn map_to_flight_status(
    raw: AviationstackFlight,
    requested_flight: &str,
    requested_origin: &str,
) -> FlightStatus {
    // Prefer upstream-supplied fields; fall back to the request's
    // own values so the response always echoes a sensible identity
    // even when the upstream partially populates rows.
    let flight_id = if raw.flight.iata.is_empty() {
        requested_flight.to_string()
    } else {
        raw.flight.iata
    };
    let origin = if raw.departure.iata.is_empty() {
        requested_origin.to_string()
    } else {
        raw.departure.iata
    };
    FlightStatus {
        flight: flight_id,
        scheduled_departure: raw.departure.scheduled,
        scheduled_arrival: raw.arrival.scheduled,
        status: if raw.flight_status.is_empty() {
            "scheduled".to_string()
        } else {
            raw.flight_status
        },
        origin,
        destination: raw.arrival.iata,
        departure_gate: raw.departure.gate,
        arrival_gate: raw.arrival.gate,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn json_body() -> serde_json::Value {
        serde_json::json!({
            "data": [{
                "flight_status": "active",
                "departure": {
                    "iata": "JFK",
                    "scheduled": "2026-04-25T12:00:00Z",
                    "gate": "A12"
                },
                "arrival": {
                    "iata": "LAX",
                    "scheduled": "2026-04-25T15:00:00Z",
                    "gate": null
                },
                "flight": { "iata": "AA100" }
            }]
        })
    }

    #[tokio::test]
    async fn fetch_flight_maps_upstream_to_flight_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/flights"))
            .and(query_param("flight_iata", "AA100"))
            .and(query_param("flight_date", "2026-04-25"))
            .and(query_param("dep_iata", "JFK"))
            .and(query_param("access_key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json_body()))
            .mount(&server)
            .await;

        let client = AviationstackClient::new(
            AviationstackConfig {
                base_url: format!("{}/v1", server.uri()),
                access_key: "test-key".into(),
            },
            reqwest::Client::new(),
        );
        let result = client
            .fetch_flight("AA100", "2026-04-25", "JFK")
            .await
            .unwrap()
            .expect("upstream returned data");
        assert_eq!(result.flight, "AA100");
        assert_eq!(result.origin, "JFK");
        assert_eq!(result.destination, "LAX");
        assert_eq!(result.status, "active");
        assert_eq!(result.scheduled_departure, "2026-04-25T12:00:00Z");
        assert_eq!(result.scheduled_arrival, "2026-04-25T15:00:00Z");
        assert_eq!(result.departure_gate.as_deref(), Some("A12"));
        assert_eq!(result.arrival_gate, None);
    }

    #[tokio::test]
    async fn fetch_flight_empty_data_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/flights"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "data": [] })),
            )
            .mount(&server)
            .await;
        let client = AviationstackClient::new(
            AviationstackConfig {
                base_url: format!("{}/v1", server.uri()),
                access_key: "k".into(),
            },
            reqwest::Client::new(),
        );
        let res = client.fetch_flight("XX1", "2026-04-25", "AAA").await.unwrap();
        assert!(res.is_none(), "empty data array must surface as Ok(None)");
    }

    #[tokio::test]
    async fn fetch_flight_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/flights"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = AviationstackClient::new(
            AviationstackConfig {
                base_url: format!("{}/v1", server.uri()),
                access_key: "k".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_flight("AA1", "2026-04-25", "JFK")
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }), "got {err:?}");
    }

    #[tokio::test]
    async fn fetch_flight_unparseable_body_yields_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/flights"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("not json {{{"),
            )
            .mount(&server)
            .await;
        let client = AviationstackClient::new(
            AviationstackConfig {
                base_url: format!("{}/v1", server.uri()),
                access_key: "k".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_flight("AA1", "2026-04-25", "JFK")
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn fetch_flight_unreachable_url_yields_io_error() {
        // 127.0.0.1:1 is reserved for tcpmux and reliably refuses
        // connections — much faster than DNS-lookup-based negative
        // tests.
        let client = AviationstackClient::new(
            AviationstackConfig {
                base_url: "http://127.0.0.1:1/v1".into(),
                access_key: "k".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_flight("AA1", "2026-04-25", "JFK")
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn map_to_flight_status_falls_back_to_request_when_upstream_omits() {
        // Drive the helper directly to lock down the fallback
        // contract: when upstream emits an empty flight.iata or
        // departure.iata, the response echoes the request fields.
        let raw = AviationstackFlight {
            flight_status: String::new(),
            departure: AviationstackEndpoint::default(),
            arrival: AviationstackEndpoint::default(),
            flight: AviationstackFlightId::default(),
        };
        let mapped = map_to_flight_status(raw, "AA100", "JFK");
        assert_eq!(mapped.flight, "AA100");
        assert_eq!(mapped.origin, "JFK");
        assert_eq!(mapped.status, "scheduled");
    }

    #[test]
    fn config_with_access_key_uses_production_base_url() {
        let cfg = AviationstackConfig::with_access_key("k");
        assert_eq!(cfg.base_url, "https://api.aviationstack.com/v1");
        assert_eq!(cfg.access_key, "k");
    }
}
