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

pub mod acled;
pub mod ais;
pub mod aviationstack;
pub mod cftc_cot;
pub mod cisa_kev;
pub mod cloudflare_radar;
pub mod coingecko;
pub mod eia;
pub mod error;
pub mod faa_notam;
pub mod gdelt;
pub mod gcp_status;
pub mod gie_agsi;
pub mod gpsjam;
pub mod hacker_news;
pub mod huggingface;
pub mod ja3;
pub mod jodi;
pub mod metaculus;
pub mod oss_insight;
pub mod vantage_compute;
pub mod nasa_eonet;
pub mod nasa_firms;
pub mod noaa_ncei;
pub mod nvd;
pub mod openaq;
pub mod opensky;
pub mod oref;
pub mod polymarket;
pub mod rss;
pub mod telegram_public;
pub mod types;
pub mod ucdp;
pub mod usgs_earthquakes;
pub mod yahoo_finance;

pub use ais::{AisClient, AisError, Backoff, WatermarkQueue, DEFAULT_CHANNEL_CAPACITY, DEFAULT_WS_URL};
pub use aviationstack::{AviationstackClient, AviationstackConfig};
pub use cftc_cot::{CftcCotClient, CftcCotConfig, CotRow};
pub use coingecko::{CoinGeckoClient, CoinGeckoConfig, CryptoQuote};
pub use error::StreamsError;
pub use faa_notam::{FaaNotamClient, FaaNotamConfig, Notam};
pub use gpsjam::{GpsjamClient, GpsjamConfig, JammingCell};
pub use nasa_eonet::{EonetConfig, EventGeometry, NasaEonetClient, NaturalEvent};
pub use nasa_firms::{FireDetection, NasaFirmsClient, NasaFirmsConfig};
pub use noaa_ncei::{
    NoaaNceiClient, NoaaNceiConfig, TemperatureAnomaly, TemperatureAnomalySeries,
};
pub use acled::{AcledClient, AcledConfig, AcledEvent};
pub use cisa_kev::{CisaKevClient, CisaKevConfig, KevCatalog, KevVulnerability};
pub use cloudflare_radar::{
    CloudflareRadarClient, CloudflareRadarConfig, OutageAnnotation,
};
pub use eia::{EiaClient, EiaConfig, EiaQuery, EiaResponse, EiaRow};
pub use gdelt::{GdeltArticle, GdeltClient, GdeltConfig};
pub use gcp_status::{GcpIncident, GcpStatusClient, GcpStatusConfig};
pub use gie_agsi::{GasStorageRow, GieAgsiClient, GieAgsiConfig};
pub use hacker_news::{HackerNewsClient, HackerNewsConfig, HnStory};
pub use huggingface::{HfModel, HfSortBy, HuggingFaceClient, HuggingFaceConfig};
pub use jodi::{JodiClient, JodiConfig, JodiDataset, JodiRow};
pub use metaculus::{MetaculusClient, MetaculusConfig, MetaculusQuestion};
pub use nvd::{NvdClient, NvdConfig, NvdResponse, NvdVulnerability};
pub use oss_insight::{OssInsightClient, OssInsightConfig, TrendingPeriod, TrendingRepo};
pub use vantage_compute::{InstancePricing, VantageComputeClient, VantageComputeConfig};
pub use polymarket::{PolymarketClient, PolymarketConfig, PredictionMarket};
pub use telegram_public::{TelegramMessage, TelegramPublicClient, TelegramPublicConfig};
pub use openaq::{AirMeasurement, AirQualityStation, OpenAqClient, OpenAqConfig};
pub use ucdp::{UcdpClient, UcdpConfig, UcdpEvent, UcdpPage};
pub use usgs_earthquakes::{
    EarthquakeEvent, FeedWindow, UsgsConfig, UsgsEarthquakesClient,
};
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
