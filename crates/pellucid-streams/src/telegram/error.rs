//! Errors emitted by the Telegram run task.

use thiserror::Error;

use super::channels::ChannelLoadError;
use super::client::GrammersClientError;
use super::session::SessionStoreError;

/// Errors the Telegram run task can produce. Returned from `run()` so
/// the calling binary can decide between "log and retry" and "exit
/// with non-zero status".
#[derive(Debug, Error)]
pub enum TelegramRunError {
    /// `pellucid-db` write failure during atomic publish.
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    /// `pellucid_seeders::atomic_publish` rejected the envelope or hit
    /// a lock / SQL error.
    #[error("publish: {0}")]
    Publish(#[from] pellucid_seeders::atomic_publish::PublishError),
    /// Vault layer rejected a session read/write (only meaningful for
    /// the `VaultSessionStore` path on the Tauri host).
    #[error("vault: {0}")]
    Vault(#[from] pellucid_core::vault::VaultError),
    /// Underlying MTProto client (grammers / tdlib) failed.
    #[error("mtproto: {0}")]
    Mtproto(#[from] GrammersClientError),
    /// Channel-set load (`data/telegram-channels.json`) failed.
    #[error("channel set: {0}")]
    ChannelSet(#[from] ChannelLoadError),
    /// `SessionStore` impl rejected the operation (e.g. `EnvSessionStore`
    /// returning `ReadOnly` when the run task tried to persist a
    /// rotated session).
    #[error("session store: {0}")]
    SessionStore(#[from] SessionStoreError),
    /// Auth required — vault has no telegram_session yet. The relay
    /// surfaces this as a fatal startup error (operator re-issues a
    /// fresh secret); the desktop sidecar surfaces it as "user has not
    /// completed onboarding yet" and idles.
    #[error("telegram auth required: no session in store")]
    AuthRequired,
    /// Empty channel set (every channel filtered out, or
    /// `TELEGRAM_CHANNEL_SET=""`).
    #[error("no channels configured for the run task")]
    NoChannels,
    /// Per-cycle empty result — every channel returned zero messages
    /// AND zero per-channel errors. Treated as a soft warning by the
    /// run loop (re-tries on next tick) rather than a fatal error.
    #[error("upstream returned no data")]
    EmptyUpstream,
}
