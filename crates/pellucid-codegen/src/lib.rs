//! pellucid-codegen — build-time enforcement of the proto-schema /
//! generated-tree invariant.
//!
//! The crate contains no runtime code. Its build script (see
//! `build.rs`) hashes every `.proto` file under the workspace's
//! `proto/` tree and exposes the digest as
//! [`PROTO_DIGEST`]. The committed generated tree at
//! `crates/pellucid-handlers/src/generated/` carries a sibling
//! `.proto.sha256` file produced by `bun run gen`; the unit test
//! [`tests::committed_digest_matches_proto_tree`] compares the two
//! and fails when they diverge — i.e. someone edited a `.proto`
//! and forgot to re-run `bun run gen`.
//!
//! Why a committed digest instead of a hashed-output check: the
//! generated Rust files contain whitespace + identifier
//! reformatting that varies across sebuf plugin versions; hashing
//! the *input* schema is the stable contract.

/// SHA-256 of the deterministic concatenation of every `.proto` +
/// `buf.{yaml,gen.yaml}` under `proto/`. Computed at build time;
/// see `build.rs` for the algorithm.
pub const PROTO_DIGEST: &str = include_str!(concat!(env!("OUT_DIR"), "/proto_digest.txt"));

/// The digest the committed generated tree was produced from. Lives
/// at `crates/pellucid-handlers/src/generated/.proto.sha256` so it
/// rides next to the generated Rust files. `bun run gen`
/// regenerates both atomically.
pub const COMMITTED_DIGEST: &str =
    include_str!("../../pellucid-handlers/src/generated/.proto.sha256");

/// True iff the committed digest matches the live proto-tree
/// digest. The unit test asserts this; downstream crates can also
/// invoke it (e.g. an `assert!` in a `pellucid-edge-bin` health
/// check) for runtime confirmation.
#[must_use]
pub fn proto_tree_in_sync() -> bool {
    PROTO_DIGEST.trim() == COMMITTED_DIGEST.trim()
}

/// Returns the crate version string from `CARGO_PKG_VERSION`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn proto_digest_is_64_hex_chars() {
        let d = PROTO_DIGEST.trim();
        assert_eq!(d.len(), 64, "SHA-256 hex must be 64 chars, got {d:?}");
        assert!(d.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn committed_digest_matches_proto_tree() {
        // If this fails: `proto/` was edited but
        // `crates/pellucid-handlers/src/generated/` was not
        // regenerated. Run `bun run gen` to regenerate both, or
        // edit the committed digest by hand if you know what you
        // are doing (rare — usually means the generated tree also
        // needs an edit).
        assert!(
            proto_tree_in_sync(),
            "proto digest drift\n  live:      {}\n  committed: {}\n\n\
             Fix: run `bun run gen` (when sebuf is wired) or update\n\
             `crates/pellucid-handlers/src/generated/.proto.sha256`\n\
             to match the live digest.",
            PROTO_DIGEST.trim(),
            COMMITTED_DIGEST.trim(),
        );
    }

    #[test]
    fn version_is_set() {
        let v = version();
        assert!(!v.is_empty());
        assert!(v.contains('.'));
    }
}
