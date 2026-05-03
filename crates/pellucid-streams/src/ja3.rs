//! JA3 fingerprint computation.
//!
//! JA3 is a TLS-handshake fingerprinting scheme defined by Salesforce
//! Engineering (<https://github.com/salesforce/ja3>). The fingerprint
//! is the lowercase hex MD5 of a comma-joined canonical string built
//! from the client's TLS `ClientHello`:
//!
//! ```text
//! SSLVersion,Ciphers,Extensions,EllipticCurves,EllipticCurvePointFormats
//! ```
//!
//! Each field is a `-`-separated list of the relevant decimal values
//! pulled from the `ClientHello`. GREASE values (RFC 8701) are
//! excluded per the canonical algorithm.
//!
//! ## What this module does
//!
//! - [`Ja3ClientHello`] is the typed input — the five fields the
//!   algorithm reads.
//! - [`canonical_string`] joins them in the canonical layout.
//! - [`fingerprint`] returns the lowercase-hex MD5 over the canonical
//!   string. Output is exactly 32 lowercase hex chars.
//!
//! ## What this module does NOT do
//!
//! - It does NOT control the TLS layer's `ClientHello` to make
//!   reqwest emit a chosen JA3. That is the job of a JA3-aware HTTP
//!   client (e.g. `rquest`, `boring`-backed rustls variants).
//!   [`crate::oref::OrefClient`] provides a `with_browser_fingerprint`
//!   extension point where production callers can swap in a
//!   JA3-spoofing client; this module supplies the *computation*
//!   half of the contract — what an outbound or observed handshake
//!   resolves to.

use md5::{Digest as _, Md5};

/// JA3 fingerprint of the Chrome-121-shaped `ClientHello` encoded
/// by [`Ja3ClientHello::chrome_121`]. This is the MD5 the
/// algorithm produces for our representative shape — it locks
/// the algorithm against regression, NOT a guarantee of live
/// byte-equality with a real Chrome 121 handshake (capturing that
/// requires a TLS-level interceptor; see the module-level docstring).
///
/// If the cipher / extension / curve list in
/// [`Ja3ClientHello::chrome_121`] is ever updated to match a fresh
/// live capture, regenerate this constant in lockstep:
///
/// ```text
/// printf '<canonical_string>' | md5sum
/// ```
pub const KNOWN_CHROME_121_JA3: &str = "cd08e31494f9531f560d64c695473da9";

/// Decoded `ClientHello` fields the JA3 algorithm consumes. All
/// fields are lists of decimal values from the wire — GREASE
/// values (RFC 8701) MUST be filtered out by the caller before
/// constructing this struct.
///
/// Encoding rules per the JA3 spec:
/// - `tls_version` is the legacy `ClientHello.client_version` field
///   (always `0x0303` = `771` for TLS 1.2 / 1.3 client hellos).
/// - `ciphers` are TLS cipher-suite IDs in the order they appear.
/// - `extensions` are extension type IDs in the order they appear.
/// - `elliptic_curves` is the `supported_groups` extension's named
///   curves.
/// - `ec_point_formats` is the `ec_point_formats` extension's
///   formats list (often `[0]` for uncompressed-only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ja3ClientHello {
    /// Legacy version field. `771` = TLS 1.2/1.3 client hellos.
    pub tls_version: u16,
    /// Cipher-suite IDs, in handshake order, GREASE-stripped.
    pub ciphers: Vec<u16>,
    /// Extension type IDs, in handshake order, GREASE-stripped.
    pub extensions: Vec<u16>,
    /// Named curves from `supported_groups`, GREASE-stripped.
    pub elliptic_curves: Vec<u16>,
    /// `ec_point_formats` values.
    pub ec_point_formats: Vec<u8>,
}

impl Ja3ClientHello {
    /// A representative Chrome-121-shaped `ClientHello` —
    /// cipher suites + extensions + curves chosen to match the
    /// publicly-documented Chrome 121 stable shape (TLS 1.3
    /// AEAD ciphers first, X25519 prioritised, standard
    /// extension order). Encoded explicitly so the unit test
    /// has a known-good input the algorithm can be regression-
    /// tested against — the resulting MD5 is locked in
    /// [`KNOWN_CHROME_121_JA3`].
    ///
    /// **Note**: byte-level equality with a *live* Chrome 121
    /// handshake requires a TLS-layer capture and a JA3-aware
    /// HTTP client (`rquest`, `boring`-rustls). This shape is
    /// the algorithm test oracle, not a guarantee.
    #[must_use]
    pub fn chrome_121() -> Self {
        Self {
            tls_version: 771,
            ciphers: vec![
                4865, 4866, 4867, 49195, 49199, 49196, 49200,
                52393, 52392, 49171, 49172, 156, 157, 47, 53,
            ],
            extensions: vec![
                0, 23, 65281, 10, 11, 35, 16, 5, 13, 18, 51,
                45, 43, 27, 17513, 21,
            ],
            elliptic_curves: vec![29, 23, 24],
            ec_point_formats: vec![0],
        }
    }
}

/// Build the canonical comma-joined string the JA3 algorithm
/// hashes. Public so tests can inspect the intermediate form.
#[must_use]
pub fn canonical_string(c: &Ja3ClientHello) -> String {
    let ciphers = join_dec(&c.ciphers);
    let extensions = join_dec(&c.extensions);
    let curves = join_dec(&c.elliptic_curves);
    let formats = join_dec_u8(&c.ec_point_formats);
    format!("{},{},{},{},{}", c.tls_version, ciphers, extensions, curves, formats)
}

/// Compute the JA3 fingerprint — lowercase hex MD5 over
/// [`canonical_string`].
#[must_use]
pub fn fingerprint(c: &Ja3ClientHello) -> String {
    let canonical = canonical_string(c);
    let digest = Md5::digest(canonical.as_bytes());
    let mut out = String::with_capacity(32);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn join_dec(values: &[u16]) -> String {
    let mut s = String::new();
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            s.push('-');
        }
        s.push_str(&v.to_string());
    }
    s
}

fn join_dec_u8(values: &[u8]) -> String {
    let mut s = String::new();
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            s.push('-');
        }
        s.push_str(&v.to_string());
    }
    s
}

/// GREASE values (RFC 8701) — sentinel cipher-suite / extension /
/// curve IDs Chrome injects to keep middleboxes honest. Stripped
/// before fingerprinting per the JA3 spec.
pub const GREASE_VALUES: &[u16] = &[
    0x0a0a, 0x1a1a, 0x2a2a, 0x3a3a, 0x4a4a, 0x5a5a, 0x6a6a, 0x7a7a,
    0x8a8a, 0x9a9a, 0xaaaa, 0xbaba, 0xcaca, 0xdada, 0xeaea, 0xfafa,
];

/// `true` iff `value` is a GREASE sentinel.
#[must_use]
pub fn is_grease(value: u16) -> bool {
    GREASE_VALUES.contains(&value)
}

/// Strip GREASE values from a slice. Returns a new `Vec` —
/// callers using a streaming parser will already be filtering
/// inline; this helper is for callers that hand us a captured
/// list.
#[must_use]
pub fn strip_grease(values: &[u16]) -> Vec<u16> {
    values.iter().copied().filter(|v| !is_grease(*v)).collect()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_lists_produce_empty_field_segments() {
        let c = Ja3ClientHello {
            tls_version: 771,
            ciphers: vec![],
            extensions: vec![],
            elliptic_curves: vec![],
            ec_point_formats: vec![],
        };
        assert_eq!(canonical_string(&c), "771,,,,");
    }

    #[test]
    fn canonical_string_uses_dash_within_fields_comma_between() {
        let c = Ja3ClientHello {
            tls_version: 771,
            ciphers: vec![4865, 4866],
            extensions: vec![0, 23],
            elliptic_curves: vec![29],
            ec_point_formats: vec![0],
        };
        assert_eq!(canonical_string(&c), "771,4865-4866,0-23,29,0");
    }

    #[test]
    fn fingerprint_is_32_lowercase_hex_chars() {
        let c = Ja3ClientHello::chrome_121();
        let fp = fingerprint(&c);
        assert_eq!(fp.len(), 32);
        assert!(
            fp.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "fingerprint should be lowercase hex: {fp}"
        );
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let c = Ja3ClientHello::chrome_121();
        let a = fingerprint(&c);
        let b = fingerprint(&c);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_changes_with_any_field() {
        let base = Ja3ClientHello::chrome_121();
        let base_fp = fingerprint(&base);

        let mut tweak = base.clone();
        tweak.tls_version = 770;
        assert_ne!(fingerprint(&tweak), base_fp);

        let mut tweak = base.clone();
        tweak.ciphers.push(9999);
        assert_ne!(fingerprint(&tweak), base_fp);

        let mut tweak = base.clone();
        tweak.extensions.push(9999);
        assert_ne!(fingerprint(&tweak), base_fp);

        let mut tweak = base.clone();
        tweak.elliptic_curves.push(9999);
        assert_ne!(fingerprint(&tweak), base_fp);

        let mut tweak = base.clone();
        tweak.ec_point_formats.push(9);
        assert_ne!(fingerprint(&tweak), base_fp);
    }

    #[test]
    fn canonical_chrome_121_fingerprint_matches_known_value() {
        // The canonical Chrome 121 ClientHello → JA3 hash.
        // Locked here so any algorithm regression (sort order,
        // delimiter mix-up, GREASE filtering bug) is loud.
        let fp = fingerprint(&Ja3ClientHello::chrome_121());
        assert_eq!(
            fp, KNOWN_CHROME_121_JA3,
            "Chrome 121 canonical JA3 drift — algorithm regression"
        );
    }

    #[test]
    fn known_md5_of_minimal_input() {
        // The canonical string for an all-empty ClientHello with
        // tls_version=771 is exactly the literal "771,,,,". Its
        // MD5 is bddda940f9963577c41d7c28b1a5f65f — confirmable
        // independently via:
        //   printf '771,,,,' | md5 -q   (BSD)
        //   printf '771,,,,' | md5sum   (GNU)
        // If this test ever fails, the underlying MD5
        // implementation is broken (and so is the algorithm
        // wrapper).
        let c = Ja3ClientHello {
            tls_version: 771,
            ciphers: vec![],
            extensions: vec![],
            elliptic_curves: vec![],
            ec_point_formats: vec![],
        };
        assert_eq!(canonical_string(&c), "771,,,,");
        assert_eq!(fingerprint(&c), "bddda940f9963577c41d7c28b1a5f65f");
    }

    #[test]
    fn is_grease_recognises_all_16_sentinels() {
        for g in GREASE_VALUES {
            assert!(is_grease(*g), "{g:#06x} should be GREASE");
        }
        // Non-sentinels.
        for v in [0u16, 1, 1234, 4865, 51, 27] {
            assert!(!is_grease(v), "{v} should not be GREASE");
        }
    }

    #[test]
    fn strip_grease_removes_only_sentinels_preserves_order() {
        let mixed = vec![4865, 0x0a0a, 4866, 0x2a2a, 47];
        assert_eq!(strip_grease(&mixed), vec![4865, 4866, 47]);
    }

    #[test]
    fn fingerprint_is_grease_sensitive_when_callers_dont_strip() {
        // The algorithm itself does NOT filter GREASE — that's
        // the caller's responsibility per the JA3 spec. We
        // verify the contract: passing a list that contains
        // GREASE produces a different fingerprint than the
        // stripped list.
        let with_grease = Ja3ClientHello {
            tls_version: 771,
            ciphers: vec![0x0a0a, 4865, 4866],
            extensions: vec![],
            elliptic_curves: vec![],
            ec_point_formats: vec![],
        };
        let stripped = Ja3ClientHello {
            ciphers: strip_grease(&with_grease.ciphers),
            ..with_grease.clone()
        };
        assert_ne!(fingerprint(&with_grease), fingerprint(&stripped));
    }
}
