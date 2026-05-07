//! Wires `pellucid_telegram::run` into the relay binary.
//!
//! Builds an [`EnvVault`] populated from the relay's [`Config`]
//! (Railway secrets), then delegates to
//! `pellucid_telegram::task::try_spawn`. Returns `None` when
//! the env-derived `SecretsBlob` has no `telegram_api_id` /
//! `telegram_api_hash` — that's the dev-mode path; the panel falls back
//! to the M4 outage state until the operator wires the secrets.

use std::sync::Arc;

use pellucid_core::vault::{EnvVault, SecretsBlob, Vault};
use pellucid_db::Pool;
use pellucid_telegram::task as telegram_task;

pub use pellucid_telegram::task::{shutdown, TelegramTaskHandle};
pub use pellucid_telegram::TelegramRunError;

use crate::config::Config;

/// Build an `EnvVault` from the relay config and pass it to the
/// streams `try_spawn` helper. Returns `None` when credentials are
/// absent (dev mode); returns `Err` only when `try_spawn` itself
/// failed (vault read, MTProto connect, session-store read).
///
/// # Errors
/// See [`TelegramRunError`].
pub async fn try_spawn(
    pool: Pool,
    cfg: &Config,
) -> Result<Option<TelegramTaskHandle>, TelegramRunError> {
    let api_id = match cfg.telegram_api_id {
        Some(id) if id != 0 => id,
        _ => return Ok(None),
    };
    let api_hash = match cfg.telegram_api_hash.clone() {
        Some(h) if !h.is_empty() => h,
        _ => return Ok(None),
    };
    let session_bytes = cfg
        .telegram_session_base64
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let blob = SecretsBlob {
        telegram_api_id: Some(api_id),
        telegram_api_hash: Some(api_hash),
        telegram_session: session_bytes,
        ..Default::default()
    };
    let vault: Arc<dyn Vault> = Arc::new(EnvVault::with_blob(blob));

    telegram_task::try_spawn(
        pool,
        vault,
        cfg.telegram_session_path.clone(),
        cfg.telegram_channels_path.clone(),
        cfg.telegram_channel_set.clone(),
    )
    .await
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::config::ConfigSource;
    use pellucid_db::open_in_memory;
    use std::io::Write as _;

    fn temp_channel_file() -> tempfile::NamedTempFile {
        let mut tf = tempfile::NamedTempFile::new().unwrap();
        tf.write_all(br#"{"channels":["a"]}"#).unwrap();
        tf
    }

    fn config_without_telegram() -> Config {
        Config::parse(&ConfigSource::default()).unwrap()
    }

    #[tokio::test]
    async fn try_spawn_returns_none_when_no_credentials() {
        let pool = open_in_memory().await.unwrap();
        let cfg = config_without_telegram();
        let result = super::try_spawn(pool, &cfg).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn try_spawn_returns_none_when_only_partial_credentials() {
        let pool = open_in_memory().await.unwrap();
        let mut cfg = config_without_telegram();
        cfg.telegram_api_id = Some(123);
        // hash missing
        let result = super::try_spawn(pool, &cfg).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn try_spawn_returns_none_when_api_id_is_zero() {
        let pool = open_in_memory().await.unwrap();
        let mut cfg = config_without_telegram();
        cfg.telegram_api_id = Some(0);
        cfg.telegram_api_hash = Some("h".into());
        let result = super::try_spawn(pool, &cfg).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods)]
    async fn try_spawn_with_full_credentials_attempts_connect() {
        // We don't actually expect a successful MTProto connect in
        // unit tests (no real DC reachable), but the call should at
        // least pass through the cred-presence check and surface a
        // non-`Mtproto::AuthRequired` error to confirm the wiring is
        // correct. Skip this test on CI by default — it requires
        // real network + real grammers session storage.
        if std::env::var("TELEGRAM_E2E").is_err() {
            return;
        }
        let pool = open_in_memory().await.unwrap();
        let tf = temp_channel_file();
        let mut cfg = config_without_telegram();
        cfg.telegram_api_id = Some(1);
        cfg.telegram_api_hash = Some("dummy".into());
        cfg.telegram_channels_path = tf.path().to_path_buf();
        let session_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        cfg.telegram_session_path = session_path.to_path_buf();
        // We expect this to either Ok(Some(_)) (connect succeeded —
        // unrealistic without real creds) or Err(_) (network /
        // grammers storage failure). What we DON'T want is Ok(None),
        // which would mean the cred check rejected our valid input.
        let res = super::try_spawn(pool, &cfg).await;
        match res {
            Ok(Some(handle)) => {
                let _ = super::shutdown(handle, std::time::Duration::from_secs(2)).await;
            }
            Ok(None) => panic!("expected creds path to attempt connect"),
            Err(_) => {} // expected without real creds
        }
    }
}
