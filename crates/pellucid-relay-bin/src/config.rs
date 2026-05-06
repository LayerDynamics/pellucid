//! Relay configuration — env-driven knobs the binary reads at
//! boot time. Mirrors `pellucid-edge-bin::config` in shape:
//! every field has a documented default, and `Config::parse`
//! takes a `ConfigSource` so tests can drive deterministic
//! tuples without touching `std::env`.

use std::time::Duration;

use thiserror::Error;

/// Default listen address — `0.0.0.0:3004` per SPEC-001 §17.5.
pub const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:3004";

/// Default DB URL — in-memory for tests. Production passes
/// the on-disk Fly volume path via `PELLUCID_DB_URL`.
pub const DEFAULT_DB_URL: &str = "sqlite::memory:";

/// Default scheduler tick budget for production — `None` means
/// "run forever". Tests pass `Some(n)` to bound runtime.
pub const DEFAULT_SCHEDULER_TICK_LIMIT: Option<u64> = None;

/// Errors `Config::parse` can surface.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Listen address could not be parsed.
    #[error("invalid PELLUCID_RELAY_LISTEN_ADDR ({raw:?}): {source}")]
    InvalidListenAddr {
        /// Raw value that failed to parse.
        raw: String,
        /// Underlying parse error.
        #[source]
        source: std::net::AddrParseError,
    },
}

/// One source-of-config tuple. The binary's `main` builds a
/// snapshot via [`ConfigSource::from_process`]; tests construct
/// it directly.
#[derive(Clone, Debug, Default)]
pub struct ConfigSource {
    /// `PELLUCID_RELAY_LISTEN_ADDR`.
    pub listen_addr: Option<String>,
    /// `PELLUCID_DB_URL`.
    pub db_url: Option<String>,
    /// `RELAY_SHARED_SECRET` — proxy auth.
    pub relay_shared_secret: Option<String>,
    /// `OPENSKY_CLIENT_ID` — OAuth2 for the upstream OpenSky API.
    pub opensky_client_id: Option<String>,
    /// `OPENSKY_CLIENT_SECRET`.
    pub opensky_client_secret: Option<String>,
    /// `AIS_API_KEY` — aisstream.io WebSocket subscribe key.
    pub ais_api_key: Option<String>,
    /// `TELEGRAM_API_ID` — Telegram MTProto API ID.
    pub telegram_api_id: Option<i32>,
    /// `TELEGRAM_API_HASH` — Telegram MTProto API hash.
    pub telegram_api_hash: Option<String>,
    /// `TELEGRAM_SESSION_BASE64` — base64-encoded SQLite session bytes
    /// for the Railway-hosted relay account. The bytes are written to
    /// `telegram_session_path` at boot.
    pub telegram_session_base64: Option<String>,
    /// `TELEGRAM_SESSION_PATH` — disk location for the SQLite session
    /// file. Defaults to `data/telegram.session`.
    pub telegram_session_path: Option<String>,
    /// `TELEGRAM_CHANNELS_PATH` — JSON channel-set file. Defaults to
    /// `data/telegram-channels.json`.
    pub telegram_channels_path: Option<String>,
    /// `TELEGRAM_CHANNEL_SET` — comma-separated env override.
    pub telegram_channel_set: Option<String>,
}

/// Resolve the listen-address source from the two env layers
/// the relay binary honours. Pure — takes already-read env
/// values so tests don't have to mutate process env.
///
/// 1. `explicit` (`PELLUCID_RELAY_LISTEN_ADDR`) — canonical
///    `host:port` knob. Wins when set.
/// 2. `port_raw` (`PORT`) — Railway / Render / Heroku / Cloud
///    Run convention; just an integer. Synthesises
///    `0.0.0.0:<PORT>` when `explicit` is unset and the value
///    parses as a `u16`.
///
/// An unparseable `PORT` falls through to `None` — the caller
/// then applies the default address rather than failing boot.
#[must_use]
pub fn resolve_listen_addr_from_env(
    explicit: Option<String>,
    port_raw: Option<String>,
) -> Option<String> {
    if let Some(addr) = explicit {
        return Some(addr);
    }
    port_raw
        .and_then(|raw| raw.parse::<u16>().ok())
        .map(|port| format!("0.0.0.0:{port}"))
}

impl ConfigSource {
    /// Build a source from the process env. Single boundary —
    /// every other code path reads `Config`. Listen-address
    /// resolution is delegated to [`resolve_listen_addr_from_env`]
    /// so the env-fallback rule is unit-testable without
    /// touching process state.
    #[must_use]
    #[allow(clippy::disallowed_methods)]
    pub fn from_process() -> Self {
        Self {
            listen_addr: resolve_listen_addr_from_env(
                std::env::var("PELLUCID_RELAY_LISTEN_ADDR").ok(),
                std::env::var("PORT").ok(),
            ),
            db_url: std::env::var("PELLUCID_DB_URL").ok(),
            relay_shared_secret: std::env::var("RELAY_SHARED_SECRET").ok(),
            opensky_client_id: std::env::var("OPENSKY_CLIENT_ID").ok(),
            opensky_client_secret: std::env::var("OPENSKY_CLIENT_SECRET").ok(),
            ais_api_key: std::env::var("AIS_API_KEY").ok(),
            telegram_api_id: std::env::var("TELEGRAM_API_ID")
                .ok()
                .and_then(|s| s.parse::<i32>().ok()),
            telegram_api_hash: std::env::var("TELEGRAM_API_HASH").ok(),
            telegram_session_base64: std::env::var("TELEGRAM_SESSION_BASE64").ok(),
            telegram_session_path: std::env::var("TELEGRAM_SESSION_PATH").ok(),
            telegram_channels_path: std::env::var("TELEGRAM_CHANNELS_PATH").ok(),
            telegram_channel_set: std::env::var("TELEGRAM_CHANNEL_SET").ok(),
        }
    }
}

/// Parsed, validated relay config.
#[derive(Clone, Debug)]
pub struct Config {
    /// Validated listen address.
    pub listen_addr: std::net::SocketAddr,
    /// SQLite URL for `pellucid_db::open`.
    pub db_url: String,
    /// Shared secret the proxy middleware enforces. `None`
    /// means dev-mode (the C1 startup gate already enforces
    /// the unauth-mode invariants).
    pub relay_shared_secret: Option<String>,
    /// OpenSky OAuth2 client id (when set, the relay binary
    /// builds an authenticated upstream client; without it,
    /// the client falls back to anonymous quota).
    pub opensky_client_id: Option<String>,
    /// OpenSky OAuth2 client secret.
    pub opensky_client_secret: Option<String>,
    /// AIS subscribe API key.
    pub ais_api_key: Option<String>,
    /// Telegram API ID (T4.5.0). When `None` the relay does not start
    /// the telegram run task — the panel falls back to the M4 outage
    /// path until secrets are wired.
    pub telegram_api_id: Option<i32>,
    /// Telegram API hash. Required when `telegram_api_id` is `Some`.
    pub telegram_api_hash: Option<String>,
    /// Base64-encoded SQLite session bytes for the relay account.
    pub telegram_session_base64: Option<String>,
    /// Disk path for the SQLite session file.
    pub telegram_session_path: std::path::PathBuf,
    /// JSON channel-set file path.
    pub telegram_channels_path: std::path::PathBuf,
    /// Optional comma-separated env override of the channel set.
    pub telegram_channel_set: Option<String>,
    /// Scheduler tick budget (`None` = unbounded).
    pub scheduler_tick_limit: Option<u64>,
    /// Background task shutdown grace period.
    pub shutdown_grace: Duration,
}

impl Config {
    /// Parse a [`ConfigSource`] into a [`Config`], applying
    /// defaults.
    ///
    /// # Errors
    /// See [`ConfigError`].
    pub fn parse(source: &ConfigSource) -> Result<Self, ConfigError> {
        let raw_addr = source.listen_addr.as_deref().unwrap_or(DEFAULT_LISTEN_ADDR);
        let listen_addr = raw_addr.parse().map_err(|e: std::net::AddrParseError| {
            ConfigError::InvalidListenAddr {
                raw: raw_addr.to_string(),
                source: e,
            }
        })?;
        let telegram_session_path = source
            .telegram_session_path
            .clone()
            .unwrap_or_else(|| "data/telegram.session".to_string());
        let telegram_channels_path = source
            .telegram_channels_path
            .clone()
            .unwrap_or_else(|| "data/telegram-channels.json".to_string());
        Ok(Self {
            listen_addr,
            db_url: source
                .db_url
                .clone()
                .unwrap_or_else(|| DEFAULT_DB_URL.to_string()),
            relay_shared_secret: source.relay_shared_secret.clone(),
            opensky_client_id: source.opensky_client_id.clone(),
            opensky_client_secret: source.opensky_client_secret.clone(),
            ais_api_key: source.ais_api_key.clone(),
            telegram_api_id: source.telegram_api_id,
            telegram_api_hash: source.telegram_api_hash.clone(),
            telegram_session_base64: source.telegram_session_base64.clone(),
            telegram_session_path: std::path::PathBuf::from(telegram_session_path),
            telegram_channels_path: std::path::PathBuf::from(telegram_channels_path),
            telegram_channel_set: source.telegram_channel_set.clone(),
            scheduler_tick_limit: DEFAULT_SCHEDULER_TICK_LIMIT,
            shutdown_grace: Duration::from_secs(5),
        })
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn resolve_listen_addr_explicit_wins_over_port() {
        let resolved =
            resolve_listen_addr_from_env(Some("127.0.0.1:9999".into()), Some("8421".into()));
        assert_eq!(resolved.as_deref(), Some("127.0.0.1:9999"));
    }

    #[test]
    fn resolve_listen_addr_synthesises_from_port_when_explicit_absent() {
        let resolved = resolve_listen_addr_from_env(None, Some("8421".into()));
        assert_eq!(resolved.as_deref(), Some("0.0.0.0:8421"));
    }

    #[test]
    fn resolve_listen_addr_returns_none_for_no_env() {
        let resolved = resolve_listen_addr_from_env(None, None);
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_listen_addr_unparseable_port_falls_through_to_none() {
        // `PORT=not-a-number` must NOT poison boot — the resolver
        // returns `None` so `Config::parse` applies the default
        // `0.0.0.0:3004`.
        let resolved = resolve_listen_addr_from_env(None, Some("not-a-number".into()));
        assert!(resolved.is_none());
        let src = ConfigSource {
            listen_addr: resolved,
            ..ConfigSource::default()
        };
        let cfg = Config::parse(&src).unwrap();
        assert_eq!(cfg.listen_addr.to_string(), "0.0.0.0:3004");
    }

    #[test]
    fn resolve_listen_addr_overflowing_port_falls_through_to_none() {
        // u16 max is 65535; 70000 must not synthesise an address.
        let resolved = resolve_listen_addr_from_env(None, Some("70000".into()));
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_listen_addr_max_u16_port_is_valid() {
        let resolved = resolve_listen_addr_from_env(None, Some("65535".into()));
        assert_eq!(resolved.as_deref(), Some("0.0.0.0:65535"));
    }

    #[test]
    fn parse_default_source_uses_documented_defaults() {
        let cfg = Config::parse(&ConfigSource::default()).unwrap();
        assert_eq!(cfg.listen_addr.to_string(), "0.0.0.0:3004");
        assert_eq!(cfg.db_url, "sqlite::memory:");
        assert!(cfg.relay_shared_secret.is_none());
        assert!(cfg.scheduler_tick_limit.is_none());
    }

    #[test]
    fn parse_invalid_listen_addr_yields_typed_error() {
        let source = ConfigSource {
            listen_addr: Some("not an address".into()),
            ..ConfigSource::default()
        };
        let err = Config::parse(&source).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidListenAddr { .. }));
    }

    #[test]
    fn parse_carries_through_secrets() {
        let source = ConfigSource {
            relay_shared_secret: Some("s3cret".into()),
            opensky_client_id: Some("client-1".into()),
            opensky_client_secret: Some("secret-1".into()),
            ais_api_key: Some("ais-key".into()),
            ..ConfigSource::default()
        };
        let cfg = Config::parse(&source).unwrap();
        assert_eq!(cfg.relay_shared_secret.as_deref(), Some("s3cret"));
        assert_eq!(cfg.opensky_client_id.as_deref(), Some("client-1"));
        assert_eq!(cfg.ais_api_key.as_deref(), Some("ais-key"));
    }

    #[test]
    fn parse_overrides_listen_addr_with_custom_port() {
        let source = ConfigSource {
            listen_addr: Some("127.0.0.1:0".into()),
            ..ConfigSource::default()
        };
        let cfg = Config::parse(&source).unwrap();
        assert_eq!(cfg.listen_addr.port(), 0);
    }
}
