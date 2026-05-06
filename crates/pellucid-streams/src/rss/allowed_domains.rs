//! RSS feed allowlist — port of `shared/rss-allowed-domains.cjs`
//! from the original WorldMonitor codebase.
//!
//! The relay's RSS proxy refuses to fetch any feed whose host is
//! not in this list. It is a defense against the relay being used
//! as an open SSRF proxy: every entry is a publisher whose RSS
//! feed pellucid actively consumes (security + intelligence
//! sources, government bulletins, vetted news outlets).
//!
//! ## Source of the list
//!
//! The original `shared/rss-allowed-domains.cjs` is not checked
//! into this rebuild. The 35 entries below are reconstructed from
//! the WorldMonitor news/intelligence panel domain set documented
//! in SPEC-001 §10 (Provider Matrix) — major government sources
//! (CISA, US-CERT, NIST, ENISA), wire services (Reuters, AP, AFP,
//! BBC), and the threat-intel feeds the cyber/intelligence panels
//! consume (Krebs, Schneier, MITRE, Microsoft Security Response,
//! Google Project Zero, etc.). Each domain is a real RSS-emitting
//! publisher.
//!
//! When the original `rss-allowed-domains.cjs` becomes mountable,
//! re-derive this list from there to lock in byte-equality with
//! the legacy roster.

/// Allowed publisher hosts. Lookup is `eq_ignore_ascii_case` so
/// `Reuters.com` and `reuters.com` collapse — RFC 1035 hosts are
/// canonically lowercase but middleboxes sometimes uppercase them.
pub const ALLOWED_DOMAINS: &[&str] = &[
    // Government / agency security advisories
    "cisa.gov",
    "us-cert.cisa.gov",
    "nist.gov",
    "csrc.nist.gov",
    "enisa.europa.eu",
    "ncsc.gov.uk",
    "cyber.gc.ca",
    "cyber.gov.au",
    // CERT teams
    "cert.org",
    "kb.cert.org",
    // Threat-intel publishers
    "krebsonsecurity.com",
    "schneier.com",
    "mitre.org",
    "msrc.microsoft.com",
    "googleprojectzero.blogspot.com",
    "blog.google",
    "googleonlinesecurity.blogspot.com",
    "blog.cloudflare.com",
    "research.checkpoint.com",
    "blog.talosintelligence.com",
    "unit42.paloaltonetworks.com",
    "thedfirreport.com",
    // Wire services
    "reuters.com",
    "apnews.com",
    "afp.com",
    "bbc.co.uk",
    "feeds.bbci.co.uk",
    "rss.cnn.com",
    "aljazeera.com",
    // Long-form / specialised intelligence
    "foreignaffairs.com",
    "csis.org",
    "rusi.org",
    "cnas.org",
    "fas.org",
    // Aviation safety
    "asn.flightsafety.org",
];

/// `true` iff `host` is in the allowlist (case-insensitive).
#[must_use]
pub fn is_allowed(host: &str) -> bool {
    ALLOWED_DOMAINS.iter().any(|d| d.eq_ignore_ascii_case(host))
}

/// Number of distinct allowlist entries.
#[must_use]
pub const fn allowed_count() -> usize {
    ALLOWED_DOMAINS.len()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn allowlist_has_expected_size() {
        // The hand-curated list is sized at 35 entries — large
        // enough to cover the real publisher set, small enough that
        // the linear scan stays free.
        assert_eq!(allowed_count(), 35);
    }

    #[test]
    fn allowlist_has_no_duplicates() {
        let set: HashSet<&&str> = ALLOWED_DOMAINS.iter().collect();
        assert_eq!(set.len(), ALLOWED_DOMAINS.len());
    }

    #[test]
    fn every_entry_is_lowercase() {
        for d in ALLOWED_DOMAINS {
            assert!(
                d.bytes().all(|b| !b.is_ascii_uppercase()),
                "{d} must be lowercase",
            );
        }
    }

    #[test]
    fn every_entry_has_a_dot() {
        // Catches accidental bare-tld entries like `gov` that would
        // permit anything under that TLD.
        for d in ALLOWED_DOMAINS {
            assert!(d.contains('.'), "{d} is not a fqdn");
        }
    }

    #[test]
    fn is_allowed_matches_lowercase() {
        assert!(is_allowed("reuters.com"));
        assert!(is_allowed("cisa.gov"));
        assert!(is_allowed("krebsonsecurity.com"));
    }

    #[test]
    fn is_allowed_is_case_insensitive() {
        assert!(is_allowed("Reuters.COM"));
        assert!(is_allowed("CISA.GOV"));
    }

    #[test]
    fn is_allowed_rejects_unlisted() {
        assert!(!is_allowed("attacker.example"));
        assert!(!is_allowed("internal.local"));
        assert!(!is_allowed(""));
        // Subdomain of an allowed host is NOT permitted — the
        // allowlist is host-exact. This protects against
        // attacker-controlled subdomains of partial-match hosts.
        assert!(!is_allowed("evil.reuters.com"));
    }

    #[test]
    fn is_allowed_rejects_path_or_port_in_host() {
        // The caller MUST pass the bare host. Defense in depth:
        // if path or port leak through, the lookup fails closed.
        assert!(!is_allowed("reuters.com/feed"));
        assert!(!is_allowed("reuters.com:8080"));
    }
}
