//! Envelope schema — the canonical wrapper around every cached data
//! payload (SPEC-001 §7.4, §24.3 M6). The atomic-publish flow writes
//! `Envelope<T>` to SQLite; the gateway's stage 11 unwraps it before
//! returning to the client when the request `Accept` indicates the bare
//! payload, or returns the envelope itself when introspection is wanted.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::time::now_ms;

/// 5 MiB hard cap on serialized envelope size, mandated by SPEC-001 §7.4.
/// Atomic publish refuses to write past this so a runaway upstream cannot
/// fill the cache row table.
pub const MAX_ENVELOPE_BYTES: usize = 5 * 1024 * 1024;

/// Validates that an encoded envelope is within [`MAX_ENVELOPE_BYTES`].
///
/// # Errors
/// Returns [`Error::PayloadTooLarge`] when `payload.len() > MAX_ENVELOPE_BYTES`.
pub fn validate_envelope_size(payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_ENVELOPE_BYTES {
        Err(Error::PayloadTooLarge {
            actual: payload.len(),
            max: MAX_ENVELOPE_BYTES,
        })
    } else {
        Ok(())
    }
}

/// The `_seed` block on every envelope. Fields mirror the canonical
/// shape of the original WorldMonitor `_seed-utils.mjs` envelope so
/// migration parity tests can compare byte-equal payloads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvelopeMeta {
    /// Epoch milliseconds of the upstream call that produced `data`.
    #[serde(rename = "fetchedAt")]
    pub fetched_at_ms: i64,
    /// Number of items in `data`, when knowable. Optional because some
    /// payloads are scalar.
    #[serde(rename = "recordCount", skip_serializing_if = "Option::is_none")]
    pub record_count: Option<i64>,
    /// Source version (e.g. "aviationstack:v1") for cross-deployment
    /// compatibility audits.
    #[serde(rename = "sourceVersion", skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    /// SeedState as a string; rendered as kebab-case
    /// ("live"/"stale"/"backup"/"seeded") to match the SQL value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

impl EnvelopeMeta {
    /// Construct an [`EnvelopeMeta`] timestamped to [`now_ms`] with all
    /// optional fields empty.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fetched_at_ms: now_ms(),
            record_count: None,
            source_version: None,
            state: None,
        }
    }
}

impl Default for EnvelopeMeta {
    fn default() -> Self {
        Self::new()
    }
}

/// Generic envelope: a `_seed` metadata block plus a `data` payload.
///
/// The default envelope shape (see [`SeedEnvelope`]) carries
/// [`serde_json::Value`] so seeders can adopt it without forcing a
/// concrete typed shape on the data layer; typed handlers can also
/// instantiate `Envelope<MyHandlerData>` for stronger guarantees.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope<T> {
    /// Metadata block — serialized as `_seed` per the spec.
    #[serde(rename = "_seed")]
    pub seed: EnvelopeMeta,
    /// Payload.
    pub data: T,
}

impl<T> Envelope<T> {
    /// Construct a new envelope timestamped to [`now_ms`] with no
    /// optional metadata set.
    pub fn new(data: T) -> Self {
        Self {
            seed: EnvelopeMeta::new(),
            data,
        }
    }

    /// Builder-style override for the record count.
    #[must_use]
    pub fn with_record_count(mut self, n: i64) -> Self {
        self.seed.record_count = Some(n);
        self
    }

    /// Builder-style override for the source version label.
    #[must_use]
    pub fn with_source_version(mut self, v: impl Into<String>) -> Self {
        self.seed.source_version = Some(v.into());
        self
    }

    /// Builder-style override for the seed state.
    #[must_use]
    pub fn with_state(mut self, state: crate::seed_meta::SeedState) -> Self {
        // SeedState is a fieldless enum with `#[serde(rename_all = "kebab-case")]`,
        // so its serialized form is always a string literal — match it
        // directly to keep this conversion infallible (no expect/panic).
        self.seed.state = Some(seed_state_to_kebab(state).to_string());
        self
    }
}

const fn seed_state_to_kebab(state: crate::seed_meta::SeedState) -> &'static str {
    use crate::seed_meta::SeedState;
    match state {
        SeedState::Live => "live",
        SeedState::Stale => "stale",
        SeedState::Backup => "backup",
        SeedState::Seeded => "seeded",
    }
}

/// The default envelope shape — opaque payload as a JSON value. Seeders
/// can use this without committing to a typed schema; typed handlers
/// can instantiate `Envelope<MyHandlerData>` directly.
pub type SeedEnvelope = Envelope<Value>;

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::seed_meta::SeedState;

    #[test]
    fn validate_envelope_size_passes_at_exactly_the_max() {
        let payload = vec![0u8; MAX_ENVELOPE_BYTES];
        assert!(validate_envelope_size(&payload).is_ok());
    }

    #[test]
    fn validate_envelope_size_rejects_one_byte_over() {
        let payload = vec![0u8; MAX_ENVELOPE_BYTES + 1];
        let err = validate_envelope_size(&payload).unwrap_err();
        match err {
            Error::PayloadTooLarge { actual, max } => {
                assert_eq!(actual, MAX_ENVELOPE_BYTES + 1);
                assert_eq!(max, MAX_ENVELOPE_BYTES);
            }
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn max_envelope_bytes_is_5_mib() {
        assert_eq!(MAX_ENVELOPE_BYTES, 5 * 1024 * 1024);
    }

    #[test]
    fn envelope_meta_new_sets_now_ms() {
        let m = EnvelopeMeta::new();
        assert!(m.fetched_at_ms > 1_577_836_800_000); // > 2020-01-01
    }

    #[test]
    fn envelope_meta_default_is_new() {
        let a = EnvelopeMeta::new();
        let b = EnvelopeMeta::default();
        // fetched_at_ms can drift by a millisecond between the two calls;
        // assert the other fields are equal and the timestamps are close.
        assert_eq!(a.record_count, b.record_count);
        assert_eq!(a.source_version, b.source_version);
        assert_eq!(a.state, b.state);
        assert!((a.fetched_at_ms - b.fetched_at_ms).abs() < 100);
    }

    #[test]
    fn envelope_new_wraps_value_with_metadata() {
        let env = Envelope::new(serde_json::json!({"k": "v"}));
        assert_eq!(env.data["k"], "v");
        assert!(env.seed.fetched_at_ms > 0);
    }

    #[test]
    fn envelope_with_record_count_chains() {
        let env = Envelope::new(serde_json::json!({})).with_record_count(7);
        assert_eq!(env.seed.record_count, Some(7));
    }

    #[test]
    fn envelope_with_source_version_chains() {
        let env = Envelope::new(serde_json::json!({})).with_source_version("aviationstack:v1");
        assert_eq!(env.seed.source_version.as_deref(), Some("aviationstack:v1"));
    }

    #[test]
    fn envelope_with_state_serializes_seedstate_as_kebab_case() {
        let env = Envelope::new(serde_json::json!({})).with_state(SeedState::Backup);
        assert_eq!(env.seed.state.as_deref(), Some("backup"));
    }

    #[test]
    fn envelope_serialization_matches_canonical_shape() {
        let env = Envelope {
            seed: EnvelopeMeta {
                fetched_at_ms: 123,
                record_count: Some(5),
                source_version: Some("v1".into()),
                state: Some("live".into()),
            },
            data: serde_json::json!({"items": [1, 2, 3]}),
        };
        let json = serde_json::to_value(&env).expect("serialize");
        assert_eq!(json["_seed"]["fetchedAt"], 123);
        assert_eq!(json["_seed"]["recordCount"], 5);
        assert_eq!(json["_seed"]["sourceVersion"], "v1");
        assert_eq!(json["_seed"]["state"], "live");
        assert_eq!(json["data"]["items"][0], 1);
    }

    #[test]
    fn envelope_omits_unset_optional_fields() {
        let env = Envelope::new(serde_json::json!({"items": []}));
        let json = serde_json::to_value(&env).expect("serialize");
        assert!(json["_seed"].get("recordCount").is_none());
        assert!(json["_seed"].get("sourceVersion").is_none());
        assert!(json["_seed"].get("state").is_none());
    }

    #[test]
    fn envelope_round_trip_through_json() {
        let env = Envelope::new(serde_json::json!({"x": 1})).with_record_count(1);
        let s = serde_json::to_string(&env).expect("serialize");
        let parsed: SeedEnvelope = serde_json::from_str(&s).expect("parse");
        assert_eq!(parsed.seed.record_count, Some(1));
        assert_eq!(parsed.data["x"], 1);
    }
}
