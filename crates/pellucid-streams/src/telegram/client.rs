//! `MtprotoClient` trait + `GrammersClient` (real `grammers-client` impl).
//!
//! The trait is the OD-6 risk hedge per SPEC-001 §32 line 1376: an
//! alternate `tdlib` impl can land later behind a feature flag without
//! touching [`super::run::run`] or the seeder/handler contract. The
//! production binary uses [`GrammersClient`]; tests use `MockClient`
//! (defined in this file) so `cargo nextest` does not perform live
//! MTProto traffic.
//!
//! Session bytes are the contents of a `grammers_session::storages::SqliteSession`
//! file on disk. The client writes the bytes to a temp path, opens the
//! SQLite session against that path, and on every `current_session_bytes`
//! re-reads the file. This is the recommended grammers persistence path
//! (`SqliteSession` is the "recommended option" per the storages docs).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use grammers_client::client::{LoginToken, PasswordToken, SignInError as GrammersSignInError};
use grammers_client::session::storages::SqliteSession;
use grammers_client::{Client, InvocationError, SenderPool};
use thiserror::Error;
use tokio::task::JoinHandle;

/// One distilled message — the trait surface exposes only the fields
/// the run-task assembles into the snapshot envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedMessage {
    /// `<channel>/<id>`.
    pub data_post: String,
    /// Permalink.
    pub url: String,
    /// ISO-8601 timestamp.
    pub datetime: String,
    /// Plain-text body (markdown stripped to plain text).
    pub text: String,
    /// View counter — already formatted (e.g. `"1.2K"`). Empty when
    /// the upstream message had no view counter (e.g. private chats).
    pub views: String,
}

/// Outcome of [`MtprotoClient::login_submit_code`].
#[derive(Debug)]
pub enum LoginCodeOutcome {
    /// Login finished — session bytes are ready to persist.
    Done,
    /// Account has 2FA enabled. Caller must follow up with
    /// [`MtprotoClient::login_submit_password`].
    NeedsPassword,
}

/// Errors emitted by [`GrammersClient`] (and any other `MtprotoClient`
/// impl). Returned as a single enum so the run task can pattern-match
/// on FloodWait without leaking grammers internals through the trait.
#[derive(Debug, Error)]
pub enum GrammersClientError {
    /// Wraps `grammers_client::InvocationError`. `is_flood_wait` /
    /// `flood_wait_seconds` extract the FloodWait detail.
    #[error("invocation: {0}")]
    Invocation(String),
    /// Sign-in failure — wraps `grammers_client::client::SignInError`.
    #[error("sign-in: {0}")]
    SignIn(String),
    /// I/O error (writing the session file, reading the file back).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Underlying SQLite session storage failed (`libsql::Error` inside
    /// grammers — surfaced as a string so we don't leak the libsql dep
    /// through our public surface).
    #[error("session storage: {0}")]
    Storage(String),
    /// Channel slug did not resolve to a public peer the account can
    /// see. Returned by `fetch_recent`.
    #[error("channel `{0}` did not resolve to a public peer")]
    ChannelNotFound(String),
    /// FloodWait was returned by the server. The run task sleeps the
    /// requested seconds and retries on the next tick.
    #[error("flood wait: {seconds}s")]
    FloodWait {
        /// Seconds the server asked us to wait.
        seconds: u32,
    },
    /// Login attempt without prior `request_login_code` — caller bug.
    #[error("login state: no LoginToken in flight; call request_login_code first")]
    NoLoginInFlight,
    /// Login attempt at password step without preceding `submit_code`
    /// returning `NeedsPassword`.
    #[error("login state: no PasswordToken in flight; call submit_code first")]
    NoPasswordInFlight,
}

impl GrammersClientError {
    /// `true` when this error is a FloodWait the run loop should sleep
    /// and retry on.
    #[must_use]
    pub fn is_flood_wait(&self) -> bool {
        matches!(self, Self::FloodWait { .. })
    }

    /// Seconds the server asked us to wait, or 0 when this is not a
    /// FloodWait error.
    #[must_use]
    pub fn flood_wait_seconds(&self) -> u32 {
        if let Self::FloodWait { seconds } = self {
            *seconds
        } else {
            0
        }
    }
}

fn map_invocation(err: InvocationError) -> GrammersClientError {
    if let InvocationError::Rpc(rpc) = &err {
        if rpc.name == "FLOOD_WAIT" {
            if let Some(seconds) = rpc.value {
                return GrammersClientError::FloodWait { seconds };
            }
        }
    }
    GrammersClientError::Invocation(err.to_string())
}

fn map_sign_in(err: GrammersSignInError) -> GrammersClientError {
    if let GrammersSignInError::Other(InvocationError::Rpc(rpc)) = &err {
        if rpc.name == "FLOOD_WAIT" {
            if let Some(seconds) = rpc.value {
                return GrammersClientError::FloodWait { seconds };
            }
        }
    }
    GrammersClientError::SignIn(format!("{err:?}"))
}

/// Trait surface every Telegram MTProto client must expose. The run
/// task and the Tauri IPC commands compose against this — the
/// `tdlib` alternate impl plugs in by satisfying the trait.
#[async_trait]
pub trait MtprotoClient: Send + Sync + std::fmt::Debug {
    /// `true` when the underlying session has a logged-in user.
    async fn is_authorized(&self) -> Result<bool, GrammersClientError>;

    /// Fetch the most recent `limit` messages for `channel`, applying
    /// `per_channel_timeout` to the network call. Resolves the channel
    /// slug via the equivalent of `client.resolve_username` first. The
    /// returned vector is newest-first (the order grammers returns
    /// `iter_messages` results).
    async fn fetch_recent(
        &self,
        channel: &str,
        limit: usize,
        per_channel_timeout: Duration,
    ) -> Result<Vec<FetchedMessage>, GrammersClientError>;

    /// Issue `auth.SendCode` for `phone`. Returns when the SMS has been
    /// dispatched. Stores the resulting `LoginToken` in the impl so a
    /// follow-up `login_submit_code` can use it.
    async fn login_request_code(&self, phone: &str) -> Result<(), GrammersClientError>;

    /// Submit the SMS code from the in-flight `LoginToken`. On
    /// `Done`, the underlying session has been authenticated and
    /// `current_session_bytes` will return the freshly minted bytes.
    async fn login_submit_code(&self, code: &str) -> Result<LoginCodeOutcome, GrammersClientError>;

    /// Submit the 2FA password against the in-flight `PasswordToken`
    /// returned by the previous `login_submit_code` call.
    async fn login_submit_password(&self, password: &str) -> Result<(), GrammersClientError>;

    /// Snapshot the current session bytes (the SQLite file contents
    /// for `GrammersClient`). Called by the run task after every
    /// successful auth event so the bytes can land in the vault.
    async fn current_session_bytes(&self) -> Result<Vec<u8>, GrammersClientError>;

    /// Graceful disconnect: tells the runner to exit. Idempotent.
    async fn shutdown(&self);
}

/// Real `grammers-client`-backed implementation of [`MtprotoClient`].
pub struct GrammersClient {
    inner: Arc<GrammersClientInner>,
}

struct GrammersClientInner {
    client: Client,
    pool_handle: grammers_client::sender::SenderPoolFatHandle,
    runner: parking_lot::Mutex<Option<JoinHandle<()>>>,
    session_path: PathBuf,
    api_id: i32,
    api_hash: String,
    login_token: parking_lot::Mutex<Option<LoginToken>>,
    password_token: parking_lot::Mutex<Option<PasswordToken>>,
}

impl std::fmt::Debug for GrammersClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrammersClient")
            .field("session_path", &self.inner.session_path)
            .field("api_id", &self.inner.api_id)
            .finish()
    }
}

impl GrammersClient {
    /// Connect to Telegram with the given API credentials and a
    /// session file at `session_path`. If `session_bytes` is `Some`,
    /// the bytes are written to `session_path` before the SQLite
    /// session is opened (this restores a previously-vaulted session).
    /// If `session_bytes` is `None`, a fresh session is created at
    /// `session_path`.
    ///
    /// # Errors
    /// [`GrammersClientError::Io`] if writing or reading the session
    /// path fails. [`GrammersClientError::Storage`] if grammers refuses
    /// to open the SQLite file.
    pub async fn connect(
        session_path: impl Into<PathBuf>,
        session_bytes: Option<&[u8]>,
        api_id: i32,
        api_hash: impl Into<String>,
    ) -> Result<Self, GrammersClientError> {
        let session_path = session_path.into();
        if let Some(parent) = session_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        if let Some(bytes) = session_bytes {
            std::fs::write(&session_path, bytes)?;
        }

        let session = Arc::new(
            SqliteSession::open(&session_path)
                .await
                .map_err(|e| GrammersClientError::Storage(e.to_string()))?,
        );

        let SenderPool {
            runner,
            handle,
            updates: _,
        } = SenderPool::new(Arc::clone(&session), api_id);

        let pool_handle = handle.clone();
        let client = Client::new(handle);
        let runner_handle = tokio::spawn(async move {
            runner.run().await;
        });

        Ok(Self {
            inner: Arc::new(GrammersClientInner {
                client,
                pool_handle,
                runner: parking_lot::Mutex::new(Some(runner_handle)),
                session_path,
                api_id,
                api_hash: api_hash.into(),
                login_token: parking_lot::Mutex::new(None),
                password_token: parking_lot::Mutex::new(None),
            }),
        })
    }

    /// Read the current session file as bytes. Used by `connect` /
    /// `current_session_bytes`. Public so integration tests can probe
    /// the file directly.
    pub fn read_session_file(path: &Path) -> Result<Vec<u8>, GrammersClientError> {
        Ok(std::fs::read(path)?)
    }
}

#[async_trait]
impl MtprotoClient for GrammersClient {
    async fn is_authorized(&self) -> Result<bool, GrammersClientError> {
        self.inner
            .client
            .is_authorized()
            .await
            .map_err(map_invocation)
    }

    async fn fetch_recent(
        &self,
        channel: &str,
        limit: usize,
        per_channel_timeout: Duration,
    ) -> Result<Vec<FetchedMessage>, GrammersClientError> {
        let client = self.inner.client.clone();
        let channel_slug = channel.to_string();
        let limit_clamped = limit.max(1);

        let work = async move {
            let resolved = client
                .resolve_username(&channel_slug)
                .await
                .map_err(map_invocation)?;
            let peer = match resolved {
                Some(peer) => peer,
                None => return Err(GrammersClientError::ChannelNotFound(channel_slug)),
            };
            let peer_ref = match peer.to_ref().await {
                Some(r) => r,
                None => return Err(GrammersClientError::ChannelNotFound(channel_slug)),
            };

            let mut iter = client.iter_messages(peer_ref).limit(limit_clamped);
            let mut out: Vec<FetchedMessage> = Vec::with_capacity(limit_clamped);
            while let Some(message) = iter.next().await.map_err(map_invocation)? {
                let id = message.id();
                let data_post = format!("{channel_slug}/{id}");
                let url = format!("https://t.me/{channel_slug}/{id}");
                let datetime = message
                    .date()
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                let text = message.text().to_string();
                let views = String::new();
                out.push(FetchedMessage {
                    data_post,
                    url,
                    datetime,
                    text,
                    views,
                });
                if out.len() >= limit_clamped {
                    break;
                }
            }
            Ok::<_, GrammersClientError>(out)
        };

        match tokio::time::timeout(per_channel_timeout, work).await {
            Ok(res) => res,
            Err(_) => Err(GrammersClientError::Invocation(format!(
                "per-channel timeout {:?} elapsed for {channel}",
                per_channel_timeout
            ))),
        }
    }

    async fn login_request_code(&self, phone: &str) -> Result<(), GrammersClientError> {
        let token = self
            .inner
            .client
            .request_login_code(phone, &self.inner.api_hash)
            .await
            .map_err(map_invocation)?;
        {
            let mut guard = self.inner.login_token.lock();
            *guard = Some(token);
        }
        {
            let mut guard = self.inner.password_token.lock();
            *guard = None;
        }
        Ok(())
    }

    async fn login_submit_code(&self, code: &str) -> Result<LoginCodeOutcome, GrammersClientError> {
        let token = {
            let mut guard = self.inner.login_token.lock();
            match guard.take() {
                Some(t) => t,
                None => return Err(GrammersClientError::NoLoginInFlight),
            }
        };
        let result = self.inner.client.sign_in(&token, code).await;
        match result {
            Ok(_user) => {
                let mut guard = self.inner.password_token.lock();
                *guard = None;
                Ok(LoginCodeOutcome::Done)
            }
            Err(GrammersSignInError::PasswordRequired(password_token)) => {
                let mut guard = self.inner.password_token.lock();
                *guard = Some(password_token);
                Ok(LoginCodeOutcome::NeedsPassword)
            }
            Err(other) => {
                // Restore the LoginToken so the caller can retry the
                // SMS code (e.g. on user typo). Telegram is fine with
                // multiple sign_in attempts using the same token.
                let mut guard = self.inner.login_token.lock();
                *guard = Some(token);
                Err(map_sign_in(other))
            }
        }
    }

    async fn login_submit_password(&self, password: &str) -> Result<(), GrammersClientError> {
        let token = {
            let mut guard = self.inner.password_token.lock();
            match guard.take() {
                Some(t) => t,
                None => return Err(GrammersClientError::NoPasswordInFlight),
            }
        };
        // `check_password` consumes the `PasswordToken`. Telegram's
        // wrong-password response invalidates the token; the caller
        // must restart with `login_request_code` to recover. That's a
        // real protocol constraint, not a wrapper limitation.
        self.inner
            .client
            .check_password(token, password.as_bytes())
            .await
            .map_err(map_sign_in)?;
        let mut login_guard = self.inner.login_token.lock();
        *login_guard = None;
        Ok(())
    }

    async fn current_session_bytes(&self) -> Result<Vec<u8>, GrammersClientError> {
        let path = self.inner.session_path.clone();
        tokio::task::spawn_blocking(move || std::fs::read(path))
            .await
            .map_err(|e| GrammersClientError::Io(std::io::Error::other(e.to_string())))?
            .map_err(GrammersClientError::Io)
    }

    async fn shutdown(&self) {
        self.inner.pool_handle.quit();
        let handle = {
            let mut guard = self.inner.runner.lock();
            guard.take()
        };
        if let Some(h) = handle {
            let _ = h.await;
        }
    }
}

impl Clone for GrammersClient {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod tests {
    use super::*;
    use parking_lot::Mutex as PlMutex;
    use std::collections::HashMap;
    use tokio::sync::Notify;

    /// Test double for `MtprotoClient`. Lets the run-task / unit tests
    /// drive deterministic scenarios (success path, FloodWait,
    /// per-channel timeout, NeedsAuth) without live MTProto traffic.
    pub(crate) struct MockClient {
        responses: PlMutex<HashMap<String, Vec<FetchedMessage>>>,
        flood_waits_remaining: PlMutex<HashMap<String, u32>>,
        timeouts: PlMutex<HashMap<String, Duration>>,
        is_authorized: PlMutex<bool>,
        session_bytes: PlMutex<Vec<u8>>,
        shutdown_signal: Arc<Notify>,
    }

    impl std::fmt::Debug for MockClient {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MockClient").finish()
        }
    }

    impl Default for MockClient {
        fn default() -> Self {
            Self {
                responses: PlMutex::new(HashMap::new()),
                flood_waits_remaining: PlMutex::new(HashMap::new()),
                timeouts: PlMutex::new(HashMap::new()),
                is_authorized: PlMutex::new(true),
                session_bytes: PlMutex::new(b"mock-session".to_vec()),
                shutdown_signal: Arc::new(Notify::new()),
            }
        }
    }

    impl MockClient {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        pub(crate) fn set_authorized(&self, authorized: bool) {
            *self.is_authorized.lock() = authorized;
        }

        pub(crate) fn set_response(&self, channel: &str, msgs: Vec<FetchedMessage>) {
            self.responses.lock().insert(channel.to_string(), msgs);
        }

        pub(crate) fn enqueue_flood_wait(&self, channel: &str, seconds: u32) {
            self.flood_waits_remaining
                .lock()
                .insert(channel.to_string(), seconds);
        }

        pub(crate) fn force_timeout_longer_than(&self, channel: &str, longer_than: Duration) {
            self.timeouts
                .lock()
                .insert(channel.to_string(), longer_than * 2);
        }

        pub(crate) fn set_session_bytes(&self, bytes: Vec<u8>) {
            *self.session_bytes.lock() = bytes;
        }
    }

    #[async_trait]
    impl MtprotoClient for MockClient {
        async fn is_authorized(&self) -> Result<bool, GrammersClientError> {
            Ok(*self.is_authorized.lock())
        }

        async fn fetch_recent(
            &self,
            channel: &str,
            limit: usize,
            per_channel_timeout: Duration,
        ) -> Result<Vec<FetchedMessage>, GrammersClientError> {
            // FloodWait simulation: pop one entry from the queue.
            let pending = {
                let mut guard = self.flood_waits_remaining.lock();
                guard.remove(channel)
            };
            if let Some(seconds) = pending {
                return Err(GrammersClientError::FloodWait { seconds });
            }
            // Forced timeout: sleep longer than the budget.
            let forced = self.timeouts.lock().get(channel).copied();
            if let Some(sleep_for) = forced {
                let res = tokio::time::timeout(per_channel_timeout, async move {
                    tokio::time::sleep(sleep_for).await;
                })
                .await;
                if res.is_err() {
                    return Err(GrammersClientError::Invocation(format!(
                        "per-channel timeout {:?} elapsed for {channel}",
                        per_channel_timeout
                    )));
                }
            }
            let msgs = self
                .responses
                .lock()
                .get(channel)
                .cloned()
                .unwrap_or_default();
            Ok(msgs.into_iter().take(limit).collect())
        }

        async fn login_request_code(&self, _phone: &str) -> Result<(), GrammersClientError> {
            Ok(())
        }

        async fn login_submit_code(
            &self,
            _code: &str,
        ) -> Result<LoginCodeOutcome, GrammersClientError> {
            *self.is_authorized.lock() = true;
            Ok(LoginCodeOutcome::Done)
        }

        async fn login_submit_password(&self, _password: &str) -> Result<(), GrammersClientError> {
            *self.is_authorized.lock() = true;
            Ok(())
        }

        async fn current_session_bytes(&self) -> Result<Vec<u8>, GrammersClientError> {
            Ok(self.session_bytes.lock().clone())
        }

        async fn shutdown(&self) {
            self.shutdown_signal.notify_waiters();
        }
    }

    fn msg(channel: &str, id: u32) -> FetchedMessage {
        FetchedMessage {
            data_post: format!("{channel}/{id}"),
            url: format!("https://t.me/{channel}/{id}"),
            datetime: format!("2026-05-04T{:02}:00:00Z", id % 24),
            text: format!("Message {id} from {channel}"),
            views: String::new(),
        }
    }

    #[tokio::test]
    async fn mock_client_returns_canned_messages_under_limit() {
        let mock = MockClient::new();
        mock.set_response("a", vec![msg("a", 1), msg("a", 2), msg("a", 3)]);
        let out = mock
            .fetch_recent("a", 2, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].data_post, "a/1");
    }

    #[tokio::test]
    async fn mock_client_returns_empty_for_unconfigured_channel() {
        let mock = MockClient::new();
        let out = mock
            .fetch_recent("nope", 5, Duration::from_secs(1))
            .await
            .unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn mock_client_emits_flood_wait_once_then_clears() {
        let mock = MockClient::new();
        mock.set_response("a", vec![msg("a", 1)]);
        mock.enqueue_flood_wait("a", 7);
        let err = mock
            .fetch_recent("a", 1, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(err.is_flood_wait());
        assert_eq!(err.flood_wait_seconds(), 7);
        // Second call succeeds.
        let out = mock
            .fetch_recent("a", 1, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
    }

    #[tokio::test]
    async fn mock_client_per_channel_timeout_returns_invocation_error() {
        let mock = MockClient::new();
        mock.force_timeout_longer_than("a", Duration::from_millis(100));
        let err = mock
            .fetch_recent("a", 1, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(matches!(err, GrammersClientError::Invocation(_)));
        assert!(!err.is_flood_wait());
    }

    #[tokio::test]
    async fn mock_client_authorization_flag_round_trips() {
        let mock = MockClient::new();
        assert!(mock.is_authorized().await.unwrap());
        mock.set_authorized(false);
        assert!(!mock.is_authorized().await.unwrap());
    }

    #[tokio::test]
    async fn mock_client_login_submit_code_authorizes() {
        let mock = MockClient::new();
        mock.set_authorized(false);
        let out = mock.login_submit_code("12345").await.unwrap();
        assert!(matches!(out, LoginCodeOutcome::Done));
        assert!(mock.is_authorized().await.unwrap());
    }

    #[tokio::test]
    async fn mock_client_session_bytes_round_trip() {
        let mock = MockClient::new();
        mock.set_session_bytes(b"new-bytes".to_vec());
        assert_eq!(
            mock.current_session_bytes().await.unwrap(),
            b"new-bytes".to_vec()
        );
    }

    #[test]
    fn flood_wait_helpers_extract_correctly() {
        let err = GrammersClientError::FloodWait { seconds: 13 };
        assert!(err.is_flood_wait());
        assert_eq!(err.flood_wait_seconds(), 13);
        let other = GrammersClientError::ChannelNotFound("x".into());
        assert!(!other.is_flood_wait());
        assert_eq!(other.flood_wait_seconds(), 0);
    }
}
