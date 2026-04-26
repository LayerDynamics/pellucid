//! Test-only RSA key pair + JWK fixture.
//!
//! Generated once per test process via `OnceLock` so each individual
//! test does not pay the ~80ms cost of a 2048-bit RSA keygen. The
//! generated PEM is fed to `jsonwebtoken::EncodingKey::from_rsa_pem`
//! for signing test tokens, and the corresponding `Jwk` is fed to
//! `DecodingKey::from_jwk` (via the `JwksCache` priming path).
//!
//! Compiled only under `#[cfg(test)]` and visible to integration
//! tests via the `pellucid_auth::test_keys` re-export below.

#![cfg(any(test, feature = "test-keys"))]
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::sync::OnceLock;

use base64::Engine;
use jsonwebtoken::jwk::Jwk;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};

/// `kid` value carried by the test fixture's signing key.
pub const FIXTURE_KID: &str = "pellucid-test-fixture-kid";

struct GeneratedKey {
    private_pem: String,
    public_pem_unused: String,
    jwk: Jwk,
}

static FIXTURE: OnceLock<GeneratedKey> = OnceLock::new();

fn build() -> &'static GeneratedKey {
    FIXTURE.get_or_init(|| {
        let mut rng = rand_for_test();
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen");
        let public = RsaPublicKey::from(&private);

        let private_pem = private
            .to_pkcs8_pem(LineEnding::LF)
            .expect("encode private pkcs8 pem")
            .to_string();
        let public_pem = public
            .to_public_key_pem(LineEnding::LF)
            .expect("encode public pkcs8 pem");

        let jwk = build_rsa_jwk(&public, FIXTURE_KID);

        GeneratedKey {
            private_pem,
            public_pem_unused: public_pem,
            jwk,
        }
    })
}

fn build_rsa_jwk(key: &RsaPublicKey, kid: &str) -> Jwk {
    let n_bytes = key.n().to_bytes_be();
    let e_bytes = key.e().to_bytes_be();
    let n = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&n_bytes);
    let e = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&e_bytes);
    let json = serde_json::json!({
        "kty": "RSA",
        "use": "sig",
        "alg": "RS256",
        "kid": kid,
        "n": n,
        "e": e,
    });
    serde_json::from_value(json).expect("compose jwk")
}

fn rand_for_test() -> rsa::rand_core::OsRng {
    // OsRng is deterministic w.r.t. the OS entropy source — the
    // resulting key is unique per test process but valid as a test
    // fixture for that process.
    rsa::rand_core::OsRng
}

/// PEM-encoded private key the tests sign with.
pub fn fixture_signing_key() -> &'static str {
    &build().private_pem
}

/// Public PEM. Reserved for diagnostics; the tests use [`fixture_jwk`]
/// for verification.
pub fn fixture_public_key_pem() -> &'static str {
    &build().public_pem_unused
}

/// `Jwk` representation of the fixture public key, with `kid =
/// `[`FIXTURE_KID`]`.
pub fn fixture_jwk() -> Jwk {
    build().jwk.clone()
}

/// Returns a freshly-built `JwkSet` containing the fixture key.
/// Convenience wrapper for callers that want to prime a verifier's
/// cache without constructing the JwkSet themselves.
pub fn fixture_jwk_set() -> jsonwebtoken::jwk::JwkSet {
    jsonwebtoken::jwk::JwkSet {
        keys: vec![fixture_jwk()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_is_stable_across_calls_within_a_process() {
        // Same process → same key (memoised via OnceLock).
        let pem1 = fixture_signing_key().to_string();
        let pem2 = fixture_signing_key().to_string();
        assert_eq!(pem1, pem2);
    }

    #[test]
    fn fixture_jwk_has_expected_fields() {
        let jwk = fixture_jwk();
        let common = &jwk.common;
        assert_eq!(common.key_id.as_deref(), Some(FIXTURE_KID));
        match &jwk.algorithm {
            jsonwebtoken::jwk::AlgorithmParameters::RSA(rsa) => {
                assert!(!rsa.n.is_empty());
                assert!(!rsa.e.is_empty());
            }
            other => panic!("expected RSA params, got {other:?}"),
        }
    }

    #[test]
    fn fixture_jwk_set_round_trips_via_jwks_find() {
        let set = fixture_jwk_set();
        assert!(set.find(FIXTURE_KID).is_some());
        assert!(set.find("nope").is_none());
    }
}
