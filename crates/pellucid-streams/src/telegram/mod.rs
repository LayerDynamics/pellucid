//! Telegram MTProto client + run task — SPEC-001 §17.5 + T4.5.0.
//!
//! Replaces the old `telegram_public` HTML scraper with a real
//! `grammers-client`-backed MTProto poller. The relay and the desktop
//! sidecar both spawn the run task; relay uses [`EnvSessionStore`]
//! (Railway secrets) and the sidecar uses [`IpcSessionStore`] (session
//! bytes pushed over stdin from the Tauri host).
//!
//! Module map (foundation pieces shipped in this commit):
//! - [`channels`] — JSON-file channel-set loader with
//!   `TELEGRAM_CHANNEL_SET` env override (per SPEC-001 §17.5 line 885).
//! - [`session`] — `SessionStore` trait + `VaultSessionStore` (Tauri
//!   host), `IpcSessionStore` (sidecar process), `EnvSessionStore`
//!   (Railway relay).
//! - [`snapshot`] — envelope shape pinned to the handler contract.
//!
//! The grammers-client wrapper (`client`), the poll loop (`run`), the
//! spawn helper (`task`), and the [`error::TelegramRunError`]
//! aggregator land in the next sub-task as part of the same T4.5.0
//! stream — this commit lands the deterministic, grammers-free
//! foundation so the modules that *do* call grammers can compose
//! against a stable trait surface (`SessionStore`, `ChannelLoadError`,
//! `TelegramIntelMinSnapshot`).

pub mod channels;
pub mod client;
pub mod error;
pub mod run;
pub mod session;
pub mod snapshot;
pub mod task;

pub use channels::{load_channel_set, ChannelLoadError, ChannelSetFile, DEFAULT_CHANNELS};
pub use client::{
    FetchedMessage, GrammersClient, GrammersClientError, LoginCodeOutcome, MtprotoClient,
};
pub use error::TelegramRunError;
pub use run::{run, TelegramRunConfig};
pub use session::{
    EnvSessionStore, IpcSessionStore, SessionEvent, SessionStore, SessionStoreError,
    VaultSessionStore, STDOUT_TELEGRAM_SESSION_PREFIX,
};
pub use snapshot::{
    TelegramIntelMinSnapshot, TelegramMessageRow, CACHE_KEY, CASCADE_GROUP,
    DEFAULT_PER_CHANNEL_LIMIT, SOURCE_VERSION, TTL,
};
pub use task::{shutdown, spawn, try_spawn, TelegramTaskHandle};

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_matches_handler_contract() {
        // The handler at pellucid_handlers::telegram::v1::feed pins this
        // exact key. Drift here breaks the panel.
        assert_eq!(CACHE_KEY, "telegram:recent-feed:v1");
    }

    #[test]
    fn source_version_distinguishes_mtproto_from_legacy_scraper() {
        // Tests + dashboards key on this string to tell which writer
        // produced the cached row.
        assert_eq!(SOURCE_VERSION, "telegram-mtproto-v1");
    }

    #[test]
    fn cascade_group_matches_legacy_seeder_so_health_cascade_unchanged() {
        // The /health cascade groups by this tag; reusing the legacy
        // value means dashboards and parity tests stay green.
        assert_eq!(CASCADE_GROUP, "intel-telegram");
    }
}
