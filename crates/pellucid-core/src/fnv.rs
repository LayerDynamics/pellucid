//! FNV-1a 64-bit hasher — used for [ETag computation in the gateway
//! pipeline (SPEC-001 §8.3 stage 13)] and any other place where a fast,
//! non-cryptographic, well-distributed hash over a byte buffer is needed.
//!
//! Reference: <http://www.isthe.com/chongo/tech/comp/fnv/index.html>.
//! 64-bit offset basis = `0xcbf29ce484222325`,
//! prime = `0x100000001b3`.

/// FNV-1a 64-bit offset basis.
pub const FNV_OFFSET_64: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
pub const FNV_PRIME_64: u64 = 0x100_0000_01b3;

/// Streaming FNV-1a hasher. Cheap to construct; deterministic across runs
/// because there is no random seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FnvHasher(u64);

impl FnvHasher {
    /// Construct a fresh hasher seeded with the canonical FNV offset basis.
    #[must_use]
    pub const fn new() -> Self {
        Self(FNV_OFFSET_64)
    }

    /// Mix one byte into the running hash.
    pub const fn write_byte(&mut self, byte: u8) {
        self.0 ^= byte as u64;
        self.0 = self.0.wrapping_mul(FNV_PRIME_64);
    }

    /// Mix every byte of `bytes` into the running hash, in order.
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_byte(b);
        }
    }

    /// Returns the current hash value without consuming the hasher.
    #[must_use]
    pub const fn finish(&self) -> u64 {
        self.0
    }

    /// One-shot helper: hash an entire byte slice and return the digest.
    #[must_use]
    pub fn hash(bytes: &[u8]) -> u64 {
        let mut h = Self::new();
        h.write(bytes);
        h.finish()
    }

    /// Returns the digest as a lowercase 16-char hex string. Convenient
    /// for ETag headers (`ETag: "<digest>"`).
    #[must_use]
    pub fn hash_hex(bytes: &[u8]) -> String {
        format!("{:016x}", Self::hash(bytes))
    }
}

impl Default for FnvHasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Reference vectors from the FNV authors' test page. Empty input must
    /// be exactly the offset basis.
    #[test]
    fn empty_input_is_offset_basis() {
        assert_eq!(FnvHasher::hash(b""), FNV_OFFSET_64);
    }

    /// Reference vector: FNV-1a 64-bit hash of "a" is `0xaf63dc4c8601ec8c`.
    #[test]
    fn known_vector_a() {
        assert_eq!(FnvHasher::hash(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    /// Reference vector: FNV-1a 64-bit hash of "foobar" is `0x85944171f73967e8`.
    #[test]
    fn known_vector_foobar() {
        assert_eq!(FnvHasher::hash(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn streaming_matches_one_shot() {
        let payload = b"the quick brown fox jumps over the lazy dog";
        let one_shot = FnvHasher::hash(payload);
        let mut h = FnvHasher::new();
        for chunk in payload.chunks(4) {
            h.write(chunk);
        }
        assert_eq!(one_shot, h.finish());
    }

    #[test]
    fn write_byte_matches_write() {
        let payload = b"streaming bytes";
        let mut a = FnvHasher::new();
        a.write(payload);
        let mut b = FnvHasher::new();
        for &byte in payload {
            b.write_byte(byte);
        }
        assert_eq!(a.finish(), b.finish());
    }

    #[test]
    fn hash_hex_is_16_chars_lowercase() {
        let hex = FnvHasher::hash_hex(b"foobar");
        assert_eq!(hex.len(), 16);
        assert_eq!(hex, hex.to_lowercase());
        assert_eq!(hex, "85944171f73967e8");
    }

    #[test]
    fn default_is_new() {
        assert_eq!(FnvHasher::default(), FnvHasher::new());
    }

    proptest::proptest! {
        /// Determinism property: hashing the same bytes twice always yields
        /// the same digest.
        #[test]
        fn determinism(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512)) {
            let a = FnvHasher::hash(&bytes);
            let b = FnvHasher::hash(&bytes);
            proptest::prop_assert_eq!(a, b);
        }

        /// Avalanche property: a single-bit flip in the input should change
        /// the output (this isn't a strong test of FNV's avalanche, just a
        /// sanity check that we aren't accidentally returning a constant).
        #[test]
        fn single_bit_flip_changes_digest(
            mut bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 1..512)
        ) {
            let original = FnvHasher::hash(&bytes);
            bytes[0] ^= 0x01;
            let flipped = FnvHasher::hash(&bytes);
            proptest::prop_assert_ne!(original, flipped);
        }
    }
}
