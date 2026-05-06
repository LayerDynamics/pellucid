#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! Wire-compat regression: every signature this crate produces must
//! match the bytes Node's `crypto.createHmac('sha256', secret)
//! .update(user_id).digest('base64')` produces for the same input.
//!
//! The fixtures below were captured from `bun run` on
//! `node:crypto` (RFC 2104 HMAC-SHA256 + standard base64). If this
//! test ever fails, the Rust port has diverged from the Node
//! original — the gateway's webhook verifier (T5.x) trusts byte
//! equality, so any drift here is a production-blocking break in
//! OP-12.
//!
//! How to regenerate (developer ergonomics, not part of CI):
//!
//! ```sh
//! bun run -e '
//!   const { createHmac } = require("node:crypto");
//!   const cases = [/* same as below */];
//!   for (const c of cases) {
//!     const sig = createHmac("sha256", c.secret).update(c.user_id).digest("base64");
//!     console.log(JSON.stringify({ ...c, sig }));
//!   }
//! '
//! ```

use pellucid_auth::{sign_user_id_hmac, verify_user_id_hmac, IdentitySigner};

/// One fixture entry. `expected_sig` is what Node produced.
struct Fixture {
    user_id: &'static str,
    secret: &'static [u8],
    expected_sig: &'static str,
}

/// All fixtures captured from Node `crypto.createHmac('sha256', ...)`.
const FIXTURES: &[Fixture] = &[
    // 1. Empty user_id and empty secret. Both legal under RFC 2104.
    Fixture {
        user_id: "",
        secret: b"",
        expected_sig: "thNnmggU2ex3L5XXeMNfxf8Wl8STcVZTxscSFEKSxa0=",
    },
    // 2. Realistic Clerk subject + 32+ byte production-shaped secret.
    Fixture {
        user_id: "user_2nXa7QabcDefGhi",
        secret: b"DODO_IDENTITY_SIGNING_SECRET_EXAMPLE_VALUE_32B",
        expected_sig: "x4lvah75zxgSYaYh+9cCqjDSJoteyD6s5mp8c7GCpxU=",
    },
    // 3. `clerk:` prefix — the format the gateway threads through
    //    `RequestIdentity::user_key()`.
    Fixture {
        user_id: "clerk:user_2nXa7QabcDefGhi",
        secret: b"another-secret",
        expected_sig: "0gPs0/WxTbY4N7pe0VevVVv/p+kFksHZpMpOQEcVqHY=",
    },
    // 4. UTF-8 in both user_id and secret. Node's createHmac defaults
    //    to UTF-8 encoding for string inputs; Rust's `as_bytes()`
    //    also produces UTF-8. They must agree.
    Fixture {
        user_id: "🔥unicode-user-✨",
        secret: "secret with spaces and 🔑".as_bytes(),
        expected_sig: "iSrnY4Fu9mwWozzRtPTt4ZxbH5t1IGDsVO9CFn8LHto=",
    },
    // 5. 1 KB user_id with a tiny secret — exercises the
    //    HMAC inner loop on multi-block input.
    Fixture {
        user_id: ONE_KB_OF_A,
        secret: b"small",
        expected_sig: "GVZQXU3G5ZxXjHqOCfkCxOOya9C0AcOlKH+Y6kbtcFw=",
    },
];

/// 1024 bytes of `'a'`, materialised as a `&'static str` by the
/// build. The `repeat` constructor is `const fn`-incompatible at the
/// time of writing, so the literal is hand-written. The length
/// invariant is asserted at the top of the test.
const ONE_KB_OF_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
     aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn fixture_constants_are_well_formed() {
    // Anchor the literal length so a future edit cannot silently
    // shorten the input and weaken the multi-block coverage.
    assert_eq!(ONE_KB_OF_A.len(), 1024);
    assert!(ONE_KB_OF_A.bytes().all(|b| b == b'a'));
}

#[test]
fn rust_signatures_match_node_byte_for_byte() {
    for (idx, fx) in FIXTURES.iter().enumerate() {
        let actual = sign_user_id_hmac(fx.user_id, fx.secret);
        assert_eq!(
            actual,
            fx.expected_sig,
            "fixture #{idx} drift — sign({user_id_dbg:?}, secret_len={secret_len}) \
             produced {actual:?}, Node produced {expected:?}",
            user_id_dbg = fx.user_id,
            secret_len = fx.secret.len(),
            actual = actual,
            expected = fx.expected_sig,
        );
    }
}

#[test]
fn rust_verify_accepts_node_produced_signatures() {
    for (idx, fx) in FIXTURES.iter().enumerate() {
        let ok = verify_user_id_hmac(fx.user_id, fx.expected_sig, fx.secret);
        assert!(
            ok,
            "fixture #{idx} verify(node_sig) returned false — Rust + Node disagree",
        );
    }
}

#[test]
fn rust_verify_rejects_node_signature_with_swapped_user_id() {
    // Cross fixtures: a sig produced for fixture[i] must NOT verify
    // against fixture[j] (different user_id and/or secret) for any
    // i != j. This locks down the "different inputs → different
    // signatures" half of the contract — strictly stronger than the
    // round-trip property.
    for (i, src) in FIXTURES.iter().enumerate() {
        for (j, dst) in FIXTURES.iter().enumerate() {
            if i == j {
                continue;
            }
            let cross = verify_user_id_hmac(dst.user_id, src.expected_sig, dst.secret);
            assert!(
                !cross,
                "fixture sig #{i} unexpectedly verified against fixture #{j}",
            );
        }
    }
}

#[test]
fn identity_signer_produces_node_compatible_signatures() {
    // Same property, but routed through the typed wrapper so we
    // catch a regression where `IdentitySigner::sign` accidentally
    // diverges from the free function (e.g. by appending a salt).
    for (idx, fx) in FIXTURES.iter().enumerate() {
        if fx.secret.is_empty() {
            // The non-test ctor refuses empty secrets; the empty-
            // secret fixture is exercised by the free-function
            // tests above. Use the test escape hatch here so we
            // still cross-check the typed surface.
            let signer = IdentitySigner::new_unchecked(fx.secret.to_vec());
            assert_eq!(
                signer.sign(fx.user_id),
                fx.expected_sig,
                "fixture #{idx} signer drift on empty secret",
            );
            assert!(signer.verify(fx.user_id, fx.expected_sig));
            continue;
        }
        let signer = IdentitySigner::new(fx.secret.to_vec()).expect("non-empty secret");
        assert_eq!(
            signer.sign(fx.user_id),
            fx.expected_sig,
            "fixture #{idx} signer drift",
        );
        assert!(
            signer.verify(fx.user_id, fx.expected_sig),
            "fixture #{idx} signer.verify(node_sig) returned false",
        );
    }
}
