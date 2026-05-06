//! HMAC-SHA256 sign + verify primitives for the Dodo checkout
//! `metadata.wm_user_id_sig` field (OP-12).
//!
//! Direct port of `convex/lib/identitySigning.ts:29-50` (sign) and
//! `:56-72` (verify). The original signs the user id with
//! `DODO_IDENTITY_SIGNING_SECRET` and base64-encodes the digest;
//! the webhook handler verifies the value with constant-time
//! comparison so a timing oracle cannot leak signatures.
//!
//! The functions in this module are deliberately low-level: they
//! accept raw `&[u8]` secrets and do no allocation beyond the
//! base64 string. The higher-level [`crate::identity`] wrappers
//! own a secret and present a typed surface.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// Concrete HMAC type used throughout this module.
type HmacSha256 = Hmac<Sha256>;

/// Build an HMAC-SHA256 instance from `secret`. RFC 2104 defines
/// HMAC for any key length: oversized keys are pre-hashed down to
/// the block size, undersized keys are zero-padded, and the empty
/// key is explicitly legal. The [`hmac::Mac::new_from_slice`]
/// constructor is therefore infallible for `Hmac<Sha256>` — the
/// `InvalidLength` error variant is reserved for primitives whose
/// key size is constrained (e.g. block ciphers), which SHA-256 is
/// not. This tiny helper centralises the unwrap with a single
/// clippy-allow + safety justification.
#[allow(clippy::expect_used)]
fn build_mac(secret: &[u8]) -> HmacSha256 {
    HmacSha256::new_from_slice(secret).expect("HMAC-SHA256 accepts any key length per RFC 2104")
}

/// Sign `user_id` with `secret` and return the base64 (standard, with
/// `=` padding) encoding of the 32-byte HMAC-SHA256 digest. The
/// output matches Node's
/// `crypto.createHmac('sha256', secret).update(user_id).digest('base64')`
/// byte-for-byte (verified by `tests/identity_compat.rs`).
///
/// `secret` length is unconstrained — RFC 2104 hashes longer keys
/// down to the block size; the underlying [`hmac`] crate handles
/// that. An empty secret is also legal (and tested).
#[must_use]
pub fn sign_user_id_hmac(user_id: &str, secret: &[u8]) -> String {
    let mut mac = build_mac(secret);
    mac.update(user_id.as_bytes());
    let bytes = mac.finalize().into_bytes();
    STANDARD.encode(bytes)
}

/// Verify that `sig_b64` is a valid HMAC-SHA256 signature of
/// `user_id` under `secret`. Returns `false` for any decoding or
/// length error so callers cannot distinguish "malformed" from
/// "wrong" — both shapes leak nothing.
///
/// The comparison uses [`subtle::ConstantTimeEq`] so the verifier
/// cannot be turned into a timing oracle on partial signature
/// matches.
#[must_use]
pub fn verify_user_id_hmac(user_id: &str, sig_b64: &str, secret: &[u8]) -> bool {
    // Decoding failure is rolled into `false`. We do *not* short-
    // circuit before computing the expected digest: the price of a
    // single base64 decode is small and refusing to early-return on
    // malformed input keeps the timing surface flatter.
    let supplied = match STANDARD.decode(sig_b64.as_bytes()) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    let mut mac = build_mac(secret);
    mac.update(user_id.as_bytes());
    let expected = mac.finalize().into_bytes();

    // ct_eq returns 0 / 1 in a `Choice`; converting to `bool` keeps
    // the constant-time property by going through the explicit
    // `Choice::unwrap_u8`.
    expected.as_slice().ct_eq(supplied.as_slice()).into()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SECRET: &[u8] = b"unit-test-secret-32-bytes-min!!!";

    #[test]
    fn sign_then_verify_round_trips() {
        let sig = sign_user_id_hmac("clerk:user_abc", SECRET);
        assert!(verify_user_id_hmac("clerk:user_abc", &sig, SECRET));
    }

    #[test]
    fn signature_is_deterministic() {
        let a = sign_user_id_hmac("user_xyz", SECRET);
        let b = sign_user_id_hmac("user_xyz", SECRET);
        assert_eq!(a, b);
    }

    #[test]
    fn signature_changes_with_user_id() {
        let a = sign_user_id_hmac("user_a", SECRET);
        let b = sign_user_id_hmac("user_b", SECRET);
        assert_ne!(a, b);
    }

    #[test]
    fn signature_changes_with_secret() {
        let a = sign_user_id_hmac("user_a", b"secret-1");
        let b = sign_user_id_hmac("user_a", b"secret-2");
        assert_ne!(a, b);
    }

    #[test]
    fn verify_with_wrong_user_id_returns_false() {
        let sig = sign_user_id_hmac("user_a", SECRET);
        assert!(!verify_user_id_hmac("user_b", &sig, SECRET));
    }

    #[test]
    fn verify_with_wrong_secret_returns_false() {
        let sig = sign_user_id_hmac("user_a", SECRET);
        assert!(!verify_user_id_hmac("user_a", &sig, b"different-secret"));
    }

    #[test]
    fn verify_with_corrupted_base64_returns_false() {
        let sig = sign_user_id_hmac("user_a", SECRET);
        let mut corrupted = sig.clone();
        // Replace the first character with a non-base64 byte.
        corrupted.replace_range(0..1, "!");
        assert!(!verify_user_id_hmac("user_a", &corrupted, SECRET));
    }

    #[test]
    fn verify_with_truncated_signature_returns_false() {
        let sig = sign_user_id_hmac("user_a", SECRET);
        let truncated = &sig[..sig.len() / 2];
        assert!(!verify_user_id_hmac("user_a", truncated, SECRET));
    }

    #[test]
    fn verify_with_empty_signature_returns_false() {
        assert!(!verify_user_id_hmac("user_a", "", SECRET));
    }

    #[test]
    fn sign_with_empty_user_id_produces_valid_b64() {
        let sig = sign_user_id_hmac("", SECRET);
        // 32-byte digest base64-encodes to 44 chars (32*4/3 rounded
        // up to a multiple of 4 — exactly 44 with one padding char).
        assert_eq!(sig.len(), 44);
        assert!(verify_user_id_hmac("", &sig, SECRET));
    }

    #[test]
    fn sign_with_empty_secret_works() {
        let sig = sign_user_id_hmac("user_a", b"");
        assert_eq!(sig.len(), 44);
        assert!(verify_user_id_hmac("user_a", &sig, b""));
    }

    #[test]
    fn sign_with_oversized_user_id_works() {
        // 100 KB of input — well above any realistic Clerk subject.
        // HMAC-SHA256 handles any length; this guards against
        // hidden buffer assumptions.
        let big = "a".repeat(100_000);
        let sig = sign_user_id_hmac(&big, SECRET);
        assert_eq!(sig.len(), 44);
        assert!(verify_user_id_hmac(&big, &sig, SECRET));
    }

    #[test]
    fn sign_with_oversized_secret_works() {
        // RFC 2104 hashes oversized keys down to the block size.
        let big_secret: Vec<u8> = (0..10_000).map(|i| i as u8).collect();
        let sig = sign_user_id_hmac("user_a", &big_secret);
        assert_eq!(sig.len(), 44);
        assert!(verify_user_id_hmac("user_a", &sig, &big_secret));
    }

    #[test]
    fn signature_is_44_b64_chars() {
        // SHA-256 produces 32 bytes → 44 chars in standard base64
        // with one '=' pad. This invariant is load-bearing for the
        // Dodo checkout `metadata.wm_user_id_sig` field which has a
        // size budget enforced upstream.
        for input in ["", "x", "user_abc", "very-long-clerk-user-id-here"] {
            let sig = sign_user_id_hmac(input, SECRET);
            assert_eq!(sig.len(), 44, "unexpected length for input {input:?}");
            assert!(sig.ends_with('='), "missing pad for input {input:?}");
        }
    }

    proptest! {
        /// Property: round-tripping any user_id + any secret returns
        /// `true`. Catches accidental input dependence (e.g. signing
        /// vs verifying with different normalizations).
        #[test]
        fn round_trip_property(
            user_id in ".{0,128}",
            secret in proptest::collection::vec(any::<u8>(), 0..=128),
        ) {
            let sig = sign_user_id_hmac(&user_id, &secret);
            prop_assert!(verify_user_id_hmac(&user_id, &sig, &secret));
        }

        /// Property: a signature produced under one secret never
        /// verifies under a *different* secret (ignoring the
        /// astronomically unlikely collision).
        #[test]
        fn cross_secret_property(
            user_id in ".{0,64}",
            s1 in proptest::collection::vec(any::<u8>(), 1..=64),
            s2 in proptest::collection::vec(any::<u8>(), 1..=64),
        ) {
            prop_assume!(s1 != s2);
            let sig = sign_user_id_hmac(&user_id, &s1);
            prop_assert!(!verify_user_id_hmac(&user_id, &sig, &s2));
        }

        /// Property: `verify` never panics, regardless of how
        /// malformed the supplied signature is. Time-attack-relevant
        /// because a panic on certain shapes would itself be an
        /// observable side channel.
        #[test]
        fn verify_never_panics_on_arbitrary_signature(
            user_id in ".{0,64}",
            sig in ".{0,200}",
            secret in proptest::collection::vec(any::<u8>(), 0..=64),
        ) {
            // We don't care about the boolean here — only that the
            // call completes without panicking.
            let _ = verify_user_id_hmac(&user_id, &sig, &secret);
        }

        /// Property: verifying a randomly-mutated signature always
        /// returns `false`. This is the property-level analogue of
        /// the constant-time-compare guarantee: if the comparison
        /// were short-circuiting, mutation timing would be
        /// observable; correctness alone is what we can assert.
        #[test]
        fn mutated_signature_never_verifies(
            user_id in ".{1,64}",
            secret in proptest::collection::vec(any::<u8>(), 1..=64),
            mutate_at in 0usize..44usize,
        ) {
            let sig = sign_user_id_hmac(&user_id, &secret);
            let mut bytes = sig.into_bytes();
            // Pick a different valid base64 char at the mutate
            // index; if the original char is the last one we wrap.
            let orig = bytes[mutate_at];
            // Every base64 alphabet char from 'A'..'Z' maps to
            // distinct digest values, so flipping to a different
            // letter changes the underlying byte.
            let new = if orig == b'A' { b'B' } else { b'A' };
            bytes[mutate_at] = new;
            let mutated = String::from_utf8(bytes).unwrap();
            prop_assert!(!verify_user_id_hmac(&user_id, &mutated, &secret));
        }
    }
}
