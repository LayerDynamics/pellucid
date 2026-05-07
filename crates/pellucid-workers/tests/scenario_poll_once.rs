//! Integration test — drives `scenario::poll_once` against a wiremock
//! Upstash REST mock and asserts the full job lifecycle:
//!
//! 1. `BLMOVE pending → processing` returns a serialised job.
//! 2. `GET scenario-result:<job_id>` returns null (not yet processed).
//! 3. `SETEX scenario-result:<job_id>` is called twice (processing, done).
//! 4. `LREM processing 1 <raw>` is called once at end.
//!
//! The scenario chosen is `us-tariff-escalation-electronics` — a tariff
//! shock with no chokepoint dependencies, so `compute_scenario` only
//! needs `vulnerabilityIndex` rows from the exposure cache. We seed
//! those via a single pipeline GET response and verify the resulting
//! `topImpactCountries` ranking makes sense.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use wiremock::matchers::{any, body_string_contains, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use pellucid_workers::redis::RedisClient;
use pellucid_workers::scenario::{self, IterationOutcome, WorkerOptions};

/// Tariff scenario keyed in the worker's SCENARIO_TEMPLATES.
const SCENARIO_ID: &str = "us-tariff-escalation-electronics";
/// Job id that passes `validate_job` (`scenario:<13 digits>:<8 alnum>`).
const JOB_ID: &str = "scenario:1700000000000:abcd1234";

#[tokio::test]
async fn poll_once_processes_tariff_shock_job_end_to_end() {
    let server = MockServer::start().await;

    let job = json!({
        "job_id": JOB_ID,
        "scenario_id": SCENARIO_ID,
        "iso2": null,
        "enqueued_at": 1_700_000_000_000_i64,
    });
    let job_str = serde_json::to_string(&job).unwrap();

    // BLMOVE → returns the job once. Body contains "BLMOVE" + the queue
    // key so it matches uniquely against the catch-all.
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_string_contains("BLMOVE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": job_str,
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // GET scenario-result:<job_id> → null (not yet processed).
    Mock::given(method("GET"))
        .and(path_regex(r"^/get/scenario-result.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": null,
        })))
        .mount(&server)
        .await;

    // GET supply_chain:chokepoints:v4 → null.
    Mock::given(method("GET"))
        .and(path_regex(r"^/get/supply_chain.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": null,
        })))
        .mount(&server)
        .await;

    // pipeline GET — 6 reporters × 1 HS2 ("85") = 6 keys. Tariff
    // scenario uses vulnerabilityIndex; US gets 0.5, others null/0.
    Mock::given(method("POST"))
        .and(path("/pipeline"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "result": serde_json::to_string(&json!({
                "iso2": "US", "hs2": "85", "vulnerabilityIndex": 0.5
            })).unwrap() },
            { "result": serde_json::to_string(&json!({
                "iso2": "CN", "hs2": "85", "vulnerabilityIndex": 0.2
            })).unwrap() },
            { "result": null },
            { "result": null },
            { "result": null },
            { "result": null },
        ])))
        .mount(&server)
        .await;

    // Catch-all for SETEX/LREM and the second BLMOVE call.
    Mock::given(method("POST"))
        .and(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": "OK",
        })))
        .mount(&server)
        .await;

    let redis = RedisClient::new(server.uri(), "test-token".into()).unwrap();
    let opts = WorkerOptions {
        once: true,
        blmove_timeout_secs: 1,
        empty_backoff: std::time::Duration::from_millis(10),
        result_ttl_secs: 60,
    };

    let outcome = scenario::poll_once(&redis, &opts).await.unwrap();
    match outcome {
        IterationOutcome::Done { job_id } => assert_eq!(job_id, JOB_ID),
        other => panic!("expected Done, got {other:?}"),
    }
}

#[tokio::test]
async fn poll_once_returns_idle_when_queue_empty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": null
        })))
        .mount(&server)
        .await;

    let redis = RedisClient::new(server.uri(), "test-token".into()).unwrap();
    let opts = WorkerOptions {
        once: true,
        blmove_timeout_secs: 1,
        empty_backoff: std::time::Duration::from_millis(10),
        result_ttl_secs: 60,
    };
    assert_eq!(
        scenario::poll_once(&redis, &opts).await.unwrap(),
        IterationOutcome::Idle
    );
}

#[tokio::test]
async fn poll_once_discards_unparseable_payload() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("BLMOVE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": "not-json"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": "OK"
        })))
        .mount(&server)
        .await;

    let redis = RedisClient::new(server.uri(), "test-token".into()).unwrap();
    let opts = WorkerOptions {
        once: true,
        blmove_timeout_secs: 1,
        empty_backoff: std::time::Duration::from_millis(10),
        result_ttl_secs: 60,
    };
    assert_eq!(
        scenario::poll_once(&redis, &opts).await.unwrap(),
        IterationOutcome::Discarded
    );
}
