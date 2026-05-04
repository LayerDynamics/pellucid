//! Edge bin configuration — env-var parsing for the public API
//! binary.
//!
//! Per SPEC-001 §11.4 the production env shape is:
//!   PELLUCID_LISTEN_ADDR=0.0.0.0:8080
//!   PELLUCID_DB_URL=sqlite:///var/lib/pellucid/edge.db
//!   AVIATIONSTACK_API_KEY=...
//! Everything else (Clerk JWKS URL, Convex secret, Dodo IDs)
//! lands in T2.7+ as those features ship.
//!
//! The validator takes a [`ConfigSource`] snapshot rather than
//! reading `std::env` directly so unit tests can drive the
//! parser deterministically. The `from_process` constructor
//! reads the live process env at boot.

use std::net::SocketAddr;

use thiserror::Error;

/// Names of every env var the parser consults — public so
/// tests can drive without typo drift.
pub mod env_names {
    /// `host:port` the binary binds. Default `0.0.0.0:8080`.
    pub const PELLUCID_LISTEN_ADDR: &str = "PELLUCID_LISTEN_ADDR";
    /// SQLx connection string. Default `sqlite::memory:` for
    /// dev / boot smoke; production sets a file path.
    pub const PELLUCID_DB_URL: &str = "PELLUCID_DB_URL";
    /// Aviationstack API key — empty when running against
    /// wiremock'd upstream (the test seed sets a fixed value).
    pub const AVIATIONSTACK_API_KEY: &str = "AVIATIONSTACK_API_KEY";
    /// Aviationstack base URL — overridden by tests to point at
    /// wiremock. Default is the production URL.
    pub const AVIATIONSTACK_BASE_URL: &str = "AVIATIONSTACK_BASE_URL";
}

/// Default listen address. Bound to `0.0.0.0:8080` per spec.
pub const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:8080";

/// Default database URL. In-memory for cold boots; production
/// always overrides.
pub const DEFAULT_DB_URL: &str = "sqlite::memory:";

/// Default aviationstack base URL — production endpoint.
pub const DEFAULT_AVIATIONSTACK_BASE_URL: &str = "https://api.aviationstack.com/v1";

/// Why parsing failed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    /// `PELLUCID_LISTEN_ADDR` was malformed.
    #[error("invalid PELLUCID_LISTEN_ADDR ({value:?}): {reason}")]
    InvalidListenAddr {
        /// Raw env value.
        value: String,
        /// Parser error message.
        reason: String,
    },
}

/// A captured env snapshot. Parsed into [`Config`] by
/// [`Config::parse`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigSource {
    /// `host:port` string, or `None` to use the default.
    pub listen_addr: Option<String>,
    /// SQLx connection string, or `None` for the default.
    pub db_url: Option<String>,
    /// Aviationstack API key.
    pub aviationstack_api_key: Option<String>,
    /// Aviationstack base URL.
    pub aviationstack_base_url: Option<String>,
}

/// Resolve the listen-address source from the two env layers
/// the edge binary honours. Pure — takes already-read env
/// values so tests don't have to mutate process env.
///
/// 1. `explicit` (`PELLUCID_LISTEN_ADDR`) — canonical
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
    /// Read the live process env. The binary's `main` calls this
    /// once at boot; tests construct `ConfigSource` directly.
    ///
    /// Listen-address resolution is delegated to
    /// [`resolve_listen_addr_from_env`] so the env-fallback rule
    /// is unit-testable without touching process state.
    #[must_use]
    #[allow(clippy::disallowed_methods)]
    pub fn from_process() -> Self {
        Self {
            listen_addr: resolve_listen_addr_from_env(
                std::env::var(env_names::PELLUCID_LISTEN_ADDR).ok(),
                std::env::var("PORT").ok(),
            ),
            db_url: std::env::var(env_names::PELLUCID_DB_URL).ok(),
            aviationstack_api_key: std::env::var(env_names::AVIATIONSTACK_API_KEY).ok(),
            aviationstack_base_url: std::env::var(env_names::AVIATIONSTACK_BASE_URL).ok(),
        }
    }
}

/// Parsed, validated config the binary actually consumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Validated socket address.
    pub listen_addr: SocketAddr,
    /// SQLx URL (string — sqlx does its own parsing).
    pub db_url: String,
    /// Aviationstack API key. Empty when wiremock-backed.
    pub aviationstack_api_key: String,
    /// Aviationstack base URL.
    pub aviationstack_base_url: String,
}

impl Config {
    /// Parse + validate a [`ConfigSource`] into a [`Config`].
    ///
    /// # Errors
    /// Returns [`ConfigError::InvalidListenAddr`] when the
    /// listen address fails to parse as a `SocketAddr`.
    pub fn parse(src: &ConfigSource) -> Result<Self, ConfigError> {
        let listen_raw = src.listen_addr.as_deref().unwrap_or(DEFAULT_LISTEN_ADDR);
        let listen_addr: SocketAddr = listen_raw.parse().map_err(|e: std::net::AddrParseError| {
            ConfigError::InvalidListenAddr {
                value: listen_raw.to_string(),
                reason: e.to_string(),
            }
        })?;
        Ok(Self {
            listen_addr,
            db_url: src
                .db_url
                .clone()
                .unwrap_or_else(|| DEFAULT_DB_URL.to_string()),
            aviationstack_api_key: src
                .aviationstack_api_key
                .clone()
                .unwrap_or_default(),
            aviationstack_base_url: src
                .aviationstack_base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_AVIATIONSTACK_BASE_URL.to_string()),
        })
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn resolve_listen_addr_explicit_wins_over_port() {
        let resolved = resolve_listen_addr_from_env(
            Some("127.0.0.1:9999".into()),
            Some("9210".into()),
        );
        assert_eq!(resolved.as_deref(), Some("127.0.0.1:9999"));
    }

    #[test]
    fn resolve_listen_addr_synthesises_from_port_when_explicit_absent() {
        let resolved = resolve_listen_addr_from_env(None, Some("9210".into()));
        assert_eq!(resolved.as_deref(), Some("0.0.0.0:9210"));
    }

    #[test]
    fn resolve_listen_addr_returns_none_for_no_env() {
        let resolved = resolve_listen_addr_from_env(None, None);
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_listen_addr_unparseable_port_falls_through_to_none() {
        let resolved =
            resolve_listen_addr_from_env(None, Some("not-a-number".into()));
        assert!(resolved.is_none());
        // Parser then applies the documented edge default.
        let src = ConfigSource {
            listen_addr: resolved,
            ..ConfigSource::default()
        };
        let cfg = Config::parse(&src).unwrap();
        assert_eq!(cfg.listen_addr.to_string(), "0.0.0.0:8080");
    }

    #[test]
    fn resolve_listen_addr_overflowing_port_falls_through_to_none() {
        let resolved = resolve_listen_addr_from_env(None, Some("70000".into()));
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_listen_addr_max_u16_port_is_valid() {
        let resolved = resolve_listen_addr_from_env(None, Some("65535".into()));
        assert_eq!(resolved.as_deref(), Some("0.0.0.0:65535"));
    }

    #[test]
    fn defaults_apply_when_source_is_empty() {
        let cfg = Config::parse(&ConfigSource::default()).unwrap();
        assert_eq!(cfg.listen_addr.to_string(), "0.0.0.0:8080");
        assert_eq!(cfg.db_url, "sqlite::memory:");
        assert_eq!(cfg.aviationstack_api_key, "");
        assert_eq!(
            cfg.aviationstack_base_url,
            "https://api.aviationstack.com/v1"
        );
    }

    #[test]
    fn explicit_listen_addr_round_trips() {
        let src = ConfigSource {
            listen_addr: Some("127.0.0.1:9001".into()),
            ..ConfigSource::default()
        };
        let cfg = Config::parse(&src).unwrap();
        assert_eq!(cfg.listen_addr.to_string(), "127.0.0.1:9001");
    }

    #[test]
    fn ipv6_listen_addr_is_accepted() {
        let src = ConfigSource {
            listen_addr: Some("[::1]:8080".into()),
            ..ConfigSource::default()
        };
        let cfg = Config::parse(&src).unwrap();
        assert!(cfg.listen_addr.is_ipv6());
    }

    #[test]
    fn malformed_listen_addr_is_rejected() {
        let src = ConfigSource {
            listen_addr: Some("not a socket addr".into()),
            ..ConfigSource::default()
        };
        let err = Config::parse(&src).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidListenAddr { .. }));
    }

    #[test]
    fn missing_port_is_rejected() {
        let src = ConfigSource {
            listen_addr: Some("127.0.0.1".into()),
            ..ConfigSource::default()
        };
        let err = Config::parse(&src).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidListenAddr { .. }));
    }

    #[test]
    fn db_url_passes_through_verbatim() {
        let src = ConfigSource {
            db_url: Some("sqlite:///var/lib/pellucid/edge.db".into()),
            ..ConfigSource::default()
        };
        let cfg = Config::parse(&src).unwrap();
        assert_eq!(cfg.db_url, "sqlite:///var/lib/pellucid/edge.db");
    }

    #[test]
    fn aviationstack_api_key_round_trips() {
        let src = ConfigSource {
            aviationstack_api_key: Some("real-key-xyz".into()),
            ..ConfigSource::default()
        };
        let cfg = Config::parse(&src).unwrap();
        assert_eq!(cfg.aviationstack_api_key, "real-key-xyz");
    }

    #[test]
    fn aviationstack_base_url_override_round_trips() {
        let src = ConfigSource {
            aviationstack_base_url: Some("http://localhost:1234/v1".into()),
            ..ConfigSource::default()
        };
        let cfg = Config::parse(&src).unwrap();
        assert_eq!(cfg.aviationstack_base_url, "http://localhost:1234/v1");
    }

    #[test]
    fn config_source_default_is_all_none() {
        let s = ConfigSource::default();
        assert!(s.listen_addr.is_none());
        assert!(s.db_url.is_none());
        assert!(s.aviationstack_api_key.is_none());
        assert!(s.aviationstack_base_url.is_none());
    }
}
