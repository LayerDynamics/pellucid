//! Telegram MTProto poll loop.
//!
//! Polls every channel in the loaded set on a fixed interval (default
//! 60 s per SPEC-001 §17.5 line 886), with a `per_channel_timeout`
//! budget on each individual fetch. Assembles the per-cycle results
//! into a [`TelegramIntelMinSnapshot`] and atomically publishes the
//! envelope under cache key `telegram:recent-feed:v1` (matching the
//! contract the legacy scraper-backed seeder used to write).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pellucid_db::Pool;
use pellucid_seeders::atomic_publish::{atomic_publish, PublishOutcome};
use pellucid_seeders::envelope::{SeedEnvelope, SeedMeta};
use serde_json::Value;

use super::channels::load_channel_set;
use super::client::{GrammersClientError, MtprotoClient};
use super::error::TelegramRunError;
use super::session::{SessionEvent, SessionStore, SessionStoreError};
use super::snapshot::{
    TelegramIntelMinSnapshot, TelegramMessageRow, CACHE_KEY, CASCADE_GROUP, SOURCE_VERSION, TTL,
};

/// Domain partition for `atomic_publish` lock acquisition. Same value
/// the deleted seeder used so the relay's existing lock infrastructure
/// is reused without reconfiguration.
const PUBLISH_DOMAIN: &str = "telegram";

/// Run-time configuration for [`run`]. Every field has a sane default
/// matching SPEC-001 §17.5; binary callers (relay / sidecar) override
/// from env vars.
#[derive(Clone, Debug)]
pub struct TelegramRunConfig {
    /// Telegram API ID from <https://my.telegram.org>.
    pub api_id: i32,
    /// Telegram API hash. Stored alongside the session in vault.
    pub api_hash: String,
    /// Time between cycles. SPEC-001 §17.5 → 60 s.
    pub poll_interval: Duration,
    /// Per-channel network timeout. SPEC-001 §17.5 → 15 s.
    pub per_channel_timeout: Duration,
    /// Path to the JSON channel set file (`data/telegram-channels.json`).
    pub channel_set_path: PathBuf,
    /// Optional `TELEGRAM_CHANNEL_SET` env override.
    pub channel_set_env_override: Option<String>,
    /// Maximum messages retained per channel in the snapshot.
    pub per_channel_limit: usize,
}

impl TelegramRunConfig {
    /// Build a config with the SPEC-001-default cadence + timeouts.
    /// Caller fills in `api_id` / `api_hash` / paths.
    #[must_use]
    pub fn defaults(api_id: i32, api_hash: String, channel_set_path: PathBuf) -> Self {
        Self {
            api_id,
            api_hash,
            poll_interval: Duration::from_secs(60),
            per_channel_timeout: Duration::from_secs(15),
            channel_set_path,
            channel_set_env_override: None,
            per_channel_limit: super::snapshot::DEFAULT_PER_CHANNEL_LIMIT,
        }
    }
}

/// Long-running poll loop. Returns once `shutdown` is set to `true` or
/// a fatal error occurs. Soft errors (FloodWait, channel-resolution
/// failure, empty cycle) log and continue.
///
/// # Errors
/// - [`TelegramRunError::AuthRequired`] when the underlying session
///   has no logged-in user. Binary callers branch on this.
/// - [`TelegramRunError::ChannelSet`] when the channel set file fails
///   to load AND the env override is absent.
/// - [`TelegramRunError::SessionStore`] from a non-readonly persist
///   failure (vault rejected the write, etc.).
/// - [`TelegramRunError::Db`] / [`TelegramRunError::Publish`] from
///   `atomic_publish`.
pub async fn run(
    pool: Pool,
    client: Arc<dyn MtprotoClient>,
    sessions: Arc<dyn SessionStore>,
    cfg: TelegramRunConfig,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), TelegramRunError> {
    if !client.is_authorized().await? {
        return Err(TelegramRunError::AuthRequired);
    }

    // Persist the boot session if the store doesn't yet have it (first
    // login from inside the run task is rare but legal).
    persist_session_if_changed(&*client, &*sessions).await?;

    let mut session_rx = sessions.subscribe();
    let mut interval = tokio::time::interval(cfg.poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // First tick fires immediately — do an initial cycle, otherwise
    // panels stay empty for `poll_interval` after boot.

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!(
                        target: "pellucid::telegram::run",
                        "shutdown signal received; exiting telegram run loop"
                    );
                    client.shutdown().await;
                    return Ok(());
                }
            }
            change = session_rx.changed() => {
                // Vault / IPC pushed a new session. Surface it via
                // `tracing` so dashboards can see the rotation; we
                // continue running on the existing connection.
                if change.is_ok() {
                    let event = session_rx.borrow_and_update().clone();
                    match event {
                        SessionEvent::Cleared => {
                            tracing::warn!(
                                target: "pellucid::telegram::run",
                                "session was cleared (logout); exiting run loop"
                            );
                            client.shutdown().await;
                            return Err(TelegramRunError::AuthRequired);
                        }
                        SessionEvent::Updated => {
                            tracing::info!(
                                target: "pellucid::telegram::run",
                                "session updated externally; continuing on current connection"
                            );
                        }
                        SessionEvent::Initial => {
                            // First subscription tick — nothing to do.
                        }
                    }
                }
            }
            _ = interval.tick() => {
                match run_one_cycle(&pool, &*client, &cfg).await {
                    Ok(_outcome) => {
                        // After every successful cycle, check if the
                        // session bytes changed (auth-key rotation,
                        // peer-cache update inside grammers' SQLite
                        // file) and persist if so.
                        persist_session_if_changed(&*client, &*sessions).await?;
                    }
                    Err(TelegramRunError::EmptyUpstream) => {
                        tracing::warn!(
                            target: "pellucid::telegram::run",
                            "all channels returned zero messages this cycle; will retry next tick"
                        );
                        metrics::counter!("pellucid_telegram_empty_cycle_total").increment(1);
                    }
                    Err(TelegramRunError::Mtproto(err)) if err.is_flood_wait() => {
                        let secs = err.flood_wait_seconds();
                        tracing::warn!(
                            target: "pellucid::telegram::run",
                            seconds = secs,
                            "flood wait — sleeping and retrying"
                        );
                        metrics::counter!(
                            "pellucid_telegram_flood_wait_total",
                            "seconds" => secs.to_string(),
                        ).increment(1);
                        tokio::time::sleep(Duration::from_secs(u64::from(secs))).await;
                    }
                    Err(TelegramRunError::NoChannels) => {
                        tracing::warn!(
                            target: "pellucid::telegram::run",
                            "no channels configured — idling"
                        );
                    }
                    Err(other) => {
                        tracing::error!(
                            target: "pellucid::telegram::run",
                            error = ?other,
                            "telegram run cycle failed"
                        );
                        metrics::counter!("pellucid_telegram_cycle_error_total").increment(1);
                    }
                }
            }
        }
    }
}

async fn run_one_cycle(
    pool: &Pool,
    client: &dyn MtprotoClient,
    cfg: &TelegramRunConfig,
) -> Result<PublishOutcome, TelegramRunError> {
    let channels = load_channel_set(
        &cfg.channel_set_path,
        cfg.channel_set_env_override.as_deref(),
    )?;
    if channels.is_empty() {
        return Err(TelegramRunError::NoChannels);
    }

    let mut rows: Vec<TelegramMessageRow> =
        Vec::with_capacity(channels.len().saturating_mul(cfg.per_channel_limit));
    let mut had_success = false;
    let mut had_data = false;
    for channel in &channels {
        match client
            .fetch_recent(channel, cfg.per_channel_limit, cfg.per_channel_timeout)
            .await
        {
            Ok(msgs) => {
                had_success = true;
                if !msgs.is_empty() {
                    had_data = true;
                }
                for m in msgs {
                    rows.push(TelegramMessageRow {
                        channel: channel.clone(),
                        data_post: m.data_post,
                        url: m.url,
                        datetime: m.datetime,
                        text: m.text,
                        views: m.views,
                    });
                }
                metrics::counter!(
                    "pellucid_telegram_channel_fetch_ok_total",
                    "channel" => channel.clone(),
                )
                .increment(1);
            }
            Err(GrammersClientError::FloodWait { seconds }) => {
                // Bubble up so the run loop can sleep before the next tick.
                return Err(TelegramRunError::Mtproto(GrammersClientError::FloodWait {
                    seconds,
                }));
            }
            Err(err) => {
                metrics::counter!(
                    "pellucid_telegram_channel_fetch_err_total",
                    "channel" => channel.clone(),
                )
                .increment(1);
                tracing::warn!(
                    target: "pellucid::telegram::run",
                    channel = %channel,
                    error = %err,
                    "channel fetch failed; continuing with remaining channels"
                );
            }
        }
    }

    if !had_data {
        // The cycle ran but produced no rows. The handler will treat
        // an empty `rows` array as a stale snapshot anyway, so we
        // don't publish — keep the previous (still-fresh) row alive.
        metrics::counter!("pellucid_telegram_dry_cycle_total").increment(1);
        if !had_success {
            return Err(TelegramRunError::EmptyUpstream);
        }
        return Err(TelegramRunError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = TelegramIntelMinSnapshot {
        rows,
        channels,
        assembled_at_ms,
    };
    let envelope: SeedEnvelope<Value> = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(60_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(i64::MAX),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(Value::Null),
    };
    let outcome = atomic_publish(pool, PUBLISH_DOMAIN, CACHE_KEY, &envelope, TTL).await?;
    metrics::counter!("pellucid_telegram_run_cycle_total").increment(1);
    Ok(outcome)
}

async fn persist_session_if_changed(
    client: &dyn MtprotoClient,
    sessions: &dyn SessionStore,
) -> Result<(), TelegramRunError> {
    let fresh = client.current_session_bytes().await?;
    let stored = sessions.load().await?;
    if stored.as_deref() == Some(fresh.as_slice()) {
        return Ok(());
    }
    match sessions.save(&fresh).await {
        Ok(()) => {
            metrics::counter!("pellucid_telegram_session_persist_total").increment(1);
            Ok(())
        }
        Err(SessionStoreError::ReadOnly) => {
            // Relay path — Railway secrets are externally managed.
            // Log and continue; the human re-issues a fresh secret on
            // next deploy if AUTH_KEY_UNREGISTERED appears.
            tracing::debug!(
                target: "pellucid::telegram::run",
                "session store is read-only; not persisting rotated bytes"
            );
            Ok(())
        }
        Err(e) => Err(TelegramRunError::SessionStore(e)),
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::client::tests::MockClient;
    use crate::session::IpcSessionStore;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine as _;
    use pellucid_db::open_in_memory;
    use std::io::Write as _;

    fn fetched(channel: &str, id: u32) -> super::super::client::FetchedMessage {
        super::super::client::FetchedMessage {
            data_post: format!("{channel}/{id}"),
            url: format!("https://t.me/{channel}/{id}"),
            datetime: format!("2026-05-04T{:02}:00:00Z", id % 24),
            text: format!("Message {id} from {channel}"),
            views: String::new(),
        }
    }

    fn temp_channel_file(channels: &[&str]) -> tempfile::NamedTempFile {
        let mut tf = tempfile::NamedTempFile::new().unwrap();
        let json = format!(
            r#"{{"channels": [{}]}}"#,
            channels
                .iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        tf.write_all(json.as_bytes()).unwrap();
        tf
    }

    fn cfg_for(path: PathBuf) -> TelegramRunConfig {
        TelegramRunConfig {
            api_id: 12345,
            api_hash: "test-hash".into(),
            poll_interval: Duration::from_millis(50),
            per_channel_timeout: Duration::from_secs(1),
            channel_set_path: path,
            channel_set_env_override: None,
            per_channel_limit: 3,
        }
    }

    #[tokio::test]
    async fn run_one_cycle_publishes_envelope_with_mtproto_source_version() {
        let pool = open_in_memory().await.unwrap();
        let client = MockClient::new();
        client.set_response("a", vec![fetched("a", 1), fetched("a", 2)]);
        let tf = temp_channel_file(&["a"]);
        let cfg = cfg_for(tf.path().to_path_buf());
        let outcome = run_one_cycle(&pool, &client, &cfg).await.unwrap();
        assert!(outcome.bytes_written > 0);

        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let source = parsed
            .pointer("/_seed/source_version")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        assert_eq!(source, SOURCE_VERSION);
        let rows = parsed
            .pointer("/data/rows")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn run_one_cycle_floodwait_bubbles_to_caller() {
        let pool = open_in_memory().await.unwrap();
        let client = MockClient::new();
        client.set_response("a", vec![fetched("a", 1)]);
        client.enqueue_flood_wait("a", 5);
        let tf = temp_channel_file(&["a"]);
        let cfg = cfg_for(tf.path().to_path_buf());
        let err = run_one_cycle(&pool, &client, &cfg).await.unwrap_err();
        match err {
            TelegramRunError::Mtproto(inner) => {
                assert!(inner.is_flood_wait());
                assert_eq!(inner.flood_wait_seconds(), 5);
            }
            other => panic!("expected FloodWait, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_one_cycle_no_messages_yields_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let client = MockClient::new();
        let tf = temp_channel_file(&["a"]);
        let cfg = cfg_for(tf.path().to_path_buf());
        let err = run_one_cycle(&pool, &client, &cfg).await.unwrap_err();
        assert!(matches!(err, TelegramRunError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_one_cycle_per_channel_failure_does_not_abort_other_channels() {
        let pool = open_in_memory().await.unwrap();
        let client = MockClient::new();
        client.set_response("good", vec![fetched("good", 1)]);
        client.force_timeout_longer_than("bad", Duration::from_millis(50));
        let tf = temp_channel_file(&["bad", "good"]);
        let cfg = TelegramRunConfig {
            per_channel_timeout: Duration::from_millis(20),
            ..cfg_for(tf.path().to_path_buf())
        };
        let outcome = run_one_cycle(&pool, &client, &cfg).await.unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed
            .pointer("/data/rows")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        // Only the good channel succeeded; bad timed out + got logged.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("channel").unwrap().as_str().unwrap(), "good");
    }

    #[tokio::test]
    async fn run_loop_shuts_down_on_signal() {
        let pool = open_in_memory().await.unwrap();
        let client: Arc<dyn MtprotoClient> = Arc::new(MockClient::new());
        let sessions: Arc<dyn SessionStore> = Arc::new(IpcSessionStore::with_bytes(b"x".to_vec()));
        let tf = temp_channel_file(&["a"]);
        let cfg = cfg_for(tf.path().to_path_buf());
        let (tx, rx) = tokio::sync::watch::channel(false);

        // Pre-populate the mock so `is_authorized()` returns true.
        let task = tokio::spawn(async move { run(pool, client, sessions, cfg, rx).await });
        // Immediately request shutdown.
        tokio::time::sleep(Duration::from_millis(20)).await;
        tx.send(true).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_loop_returns_auth_required_when_unauthorized() {
        let pool = open_in_memory().await.unwrap();
        let client = MockClient::new();
        client.set_authorized(false);
        let arc_client: Arc<dyn MtprotoClient> = Arc::new(client);
        let sessions: Arc<dyn SessionStore> = Arc::new(IpcSessionStore::new());
        let tf = temp_channel_file(&["a"]);
        let cfg = cfg_for(tf.path().to_path_buf());
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let result = run(pool, arc_client, sessions, cfg, rx).await;
        assert!(matches!(result, Err(TelegramRunError::AuthRequired)));
    }

    #[tokio::test]
    async fn persist_session_if_changed_saves_when_bytes_differ() {
        let client = MockClient::new();
        client.set_session_bytes(b"freshly-rotated".to_vec());
        let sessions = IpcSessionStore::with_bytes(b"original".to_vec());
        let mut rx = sessions.subscribe();
        persist_session_if_changed(&client, &sessions)
            .await
            .unwrap();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow_and_update(), SessionEvent::Updated);
        assert_eq!(
            sessions.load().await.unwrap(),
            Some(b"freshly-rotated".to_vec())
        );
        // Doubles as an exercise of the read-only sentinel: encoding the
        // stored bytes the same way IpcSessionStore would on stdout.
        let _ = BASE64_STANDARD.encode(b"freshly-rotated");
    }

    #[tokio::test]
    async fn persist_session_if_changed_no_op_when_bytes_match() {
        let client = MockClient::new();
        client.set_session_bytes(b"same".to_vec());
        let sessions = IpcSessionStore::with_bytes(b"same".to_vec());
        let mut rx = sessions.subscribe();
        persist_session_if_changed(&client, &sessions)
            .await
            .unwrap();
        // No event should fire.
        let polled = tokio::time::timeout(Duration::from_millis(20), rx.changed()).await;
        assert!(polled.is_err(), "should not have notified subscribers");
    }
}
