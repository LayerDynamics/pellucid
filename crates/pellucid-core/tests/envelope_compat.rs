//! Integration test: round-trip a known-good envelope JSON fixture through
//! `Envelope<Value>` (a.k.a. `SeedEnvelope`) and assert the canonical
//! shape is preserved (field names, kebab-case state, omitted optionals).
//!
//! The fixture is a pruned excerpt of an aviation/v1/get-flight-status
//! envelope from the original WorldMonitor cache; if this test breaks we
//! lost wire-format compat and any cross-deployment cache reads will
//! refuse to deserialize.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use pellucid_core::{Envelope, SeedEnvelope};
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/envelope.json");

#[test]
fn fixture_parses_into_seed_envelope() {
    let env: SeedEnvelope = serde_json::from_str(FIXTURE).expect("parse fixture");
    assert_eq!(env.seed.fetched_at_ms, 1_726_764_000_000);
    assert_eq!(env.seed.record_count, Some(3));
    assert_eq!(env.seed.source_version.as_deref(), Some("aviationstack:v1"));
    assert_eq!(env.seed.state.as_deref(), Some("live"));
}

#[test]
fn fixture_preserves_flight_ids_and_statuses() {
    let env: SeedEnvelope = serde_json::from_str(FIXTURE).expect("parse");
    let flights = env
        .data
        .get("flights")
        .and_then(Value::as_array)
        .expect("flights array");
    assert_eq!(flights.len(), 3);
    let ids: Vec<&str> = flights
        .iter()
        .filter_map(|f: &Value| f.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(ids, vec!["AA100", "DL42", "UA8"]);
}

#[test]
fn fixture_round_trip_is_byte_compatible_for_field_names() {
    let env: SeedEnvelope = serde_json::from_str(FIXTURE).expect("parse");
    let reserialized = serde_json::to_value(&env).expect("serialize");
    // Camel-case wire keys must round-trip unchanged.
    assert!(reserialized["_seed"].get("fetchedAt").is_some());
    assert!(reserialized["_seed"].get("recordCount").is_some());
    assert!(reserialized["_seed"].get("sourceVersion").is_some());
    assert_eq!(reserialized["_seed"]["state"], "live");
}

#[test]
fn typed_envelope_wraps_a_concrete_struct() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Flight {
        id: String,
        status: String,
    }
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Payload {
        flights: Vec<Flight>,
    }
    let typed: Envelope<Payload> = serde_json::from_str(FIXTURE).expect("typed parse");
    assert_eq!(typed.data.flights.len(), 3);
    assert_eq!(typed.data.flights[0].id, "AA100");
    assert_eq!(typed.data.flights[1].status, "scheduled");
}
