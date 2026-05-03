//! pellucid-streams — upstream HTTP clients for aviationstack, AIS,
//! OpenSky, RSS, Telegram, OREF, and the rest of the SPEC-001 §10
//! provider matrix.
//!
//! Each provider has its own module exposing typed `fetch_*`
//! functions that take an injected `reqwest::Client` + base URL so
//! integration tests can point them at a `wiremock` server. The
//! production binaries (`pellucid-edge-bin`, `pellucid-relay-bin`,
//! `pellucid-seeders-bin`) build a single client per process and
//! pass it through. Errors are wrapped in [`StreamsError`] so the
//! gateway's stage 11 can map them to consistent envelope shapes.

pub mod ais;
pub mod aviationstack;
pub mod cftc_cot;
pub mod coingecko;
pub mod error;
pub mod ja3;
pub mod opensky;
pub mod oref;
pub mod rss;
pub mod types;
pub mod yahoo_finance;

pub use ais::{AisClient, AisError, Backoff, WatermarkQueue, DEFAULT_CHANNEL_CAPACITY, DEFAULT_WS_URL};
pub use aviationstack::{AviationstackClient, AviationstackConfig};
pub use cftc_cot::{CftcCotClient, CftcCotConfig, CotRow};
pub use coingecko::{CoinGeckoClient, CoinGeckoConfig, CryptoQuote};
pub use error::StreamsError;
pub use ja3::{fingerprint as ja3_fingerprint, Ja3ClientHello, KNOWN_CHROME_121_JA3};
pub use opensky::{OpenSkyClient, OpenSkyConfig, OpenSkyResponse};
pub use oref::{OrefAlert, OrefClient, OrefConfig, OrefHistory, HISTORY_CACHE_KEY};
pub use rss::{RssClient, RssEntry, RssFeed, NEGATIVE_TTL, POSITIVE_TTL};
pub use types::{AisEnvelope, AisMetadata, AisSubscribe};
pub use yahoo_finance::{YahooFinanceClient, YahooFinanceConfig, YahooQuote};

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
    fn version_is_set() {
        let v = version();
        assert!(!v.is_empty(), "version must not be empty");
        assert!(v.contains('.'), "expected semver with dot, got {v}");
    }

    #[test]
    fn version_matches_workspace() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
