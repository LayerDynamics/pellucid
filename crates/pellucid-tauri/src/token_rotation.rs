//! H1 fix — sidecar bearer-token rotation.
//!
//! The original WorldMonitor desktop host minted a single sidecar bearer
//! token at boot and never rotated it. SPEC-001 §10.3 (H1) requires a
//! 5-minute rotation cadence with a 30-second overlap during which the
//! sidecar accepts both the current and the previous token. This module
//! implements the host-side rotator; the sidecar's matching acceptance
//! check ships in T1.9.
//!
//! Design
//! - [`Clock`] is an injectable monotonic clock so tests can advance time
//!   without sleeping. [`SystemClock`] uses `Instant`; [`ManualClock`]
//!   exposes [`ManualClock::advance`] for unit tests and the H1
//!   regression test (`tests/regression_h1.rs`).
//! - [`TokenRotator`] owns the rotation state behind a `parking_lot::RwLock`.
//!   It exposes `current`, `previous`, `accepts`, and `rotate_now`.
//! - [`spawn_rotation_loop`] runs the production tokio loop that calls
//!   `rotate_now` every 5 minutes and invokes a user-supplied callback so
//!   the host can emit the Tauri `token_rotated` event and persist the
//!   new token into the vault.
//!
//! Tokens are 32 cryptographically random bytes encoded as 64 hex
//! characters. Generation goes through `getrandom` rather than a PRNG so
//! the production path is OS-RNG only — no opportunity for a weak seed.

use std::fmt::{self, Debug};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use thiserror::Error;
use tokio::task::JoinHandle;

/// 5 minutes — SPEC-001 §10.3 default rotation cadence.
pub const DEFAULT_ROTATION_INTERVAL_MS: u64 = 5 * 60 * 1_000;
/// 30 seconds — overlap window during which the previous token is still
/// accepted. SPEC-001 §10.3.
pub const DEFAULT_OVERLAP_MS: u64 = 30 * 1_000;
/// 32 bytes of OS entropy → 64 hex characters.
pub const TOKEN_BYTES: usize = 32;

/// Monotonic clock abstraction. Implementations must be `Send + Sync`
/// because the rotation loop runs on a tokio task.
pub trait Clock: Send + Sync + Debug {
    /// Milliseconds since the clock's reference point. The reference
    /// point is implementation-defined; the only contract is that the
    /// value is monotonically non-decreasing.
    fn monotonic_ms(&self) -> u64;
}

/// Production clock backed by `std::time::Instant`.
#[derive(Clone, Debug)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    /// Construct a clock anchored at the moment of construction.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn monotonic_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// Manual clock used by tests. Starts at zero; `advance` bumps it.
#[derive(Debug)]
pub struct ManualClock {
    now_ms: AtomicU64,
}

impl ManualClock {
    /// Construct a manual clock starting at `t = 0 ms`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            now_ms: AtomicU64::new(0),
        }
    }

    /// Construct a manual clock starting at `t = start_ms`.
    #[must_use]
    pub fn at(start_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(start_ms),
        }
    }

    /// Advance the clock by `delta_ms`. Returns the new time.
    pub fn advance(&self, delta_ms: u64) -> u64 {
        self.now_ms.fetch_add(delta_ms, Ordering::SeqCst) + delta_ms
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn monotonic_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

/// Errors raised when generating a fresh token.
#[derive(Debug, Error)]
pub enum TokenGenError {
    /// `getrandom` failed — typically only happens in environments with
    /// no entropy source (e.g. early boot).
    #[error("os entropy source failed: {0}")]
    Os(#[from] getrandom::Error),
}

/// Generate a fresh 32-byte hex bearer token using OS entropy.
pub fn generate_token() -> Result<String, TokenGenError> {
    let mut buf = [0u8; TOKEN_BYTES];
    getrandom::getrandom(&mut buf)?;
    let mut out = String::with_capacity(TOKEN_BYTES * 2);
    for byte in &buf {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

/// Snapshot returned by [`TokenRotator::rotate_now`] so callers can
/// persist the new token, emit Tauri events, etc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotationOutcome {
    /// The fresh bearer token that just became current.
    pub new_token: String,
    /// The token that just got demoted to "previous" (still accepted
    /// during the overlap window).
    pub retired_token: String,
    /// Time the rotation occurred, on the rotator's clock.
    pub at_ms: u64,
}

#[derive(Debug)]
struct RotationState {
    current_token: String,
    previous_token: Option<String>,
    previous_retired_at_ms: Option<u64>,
    last_rotation_ms: u64,
}

/// Rotator owning the current + previous token, plus the rotation
/// schedule. Cheap to clone via `Arc`.
#[derive(Debug)]
pub struct TokenRotator {
    clock: Arc<dyn Clock>,
    state: RwLock<RotationState>,
    rotation_interval_ms: u64,
    overlap_ms: u64,
}

impl TokenRotator {
    /// Construct a rotator seeded with `initial_token`. Uses the SPEC
    /// defaults (5-minute interval, 30-second overlap).
    #[must_use]
    pub fn new(initial_token: String, clock: Arc<dyn Clock>) -> Self {
        let now = clock.monotonic_ms();
        Self {
            clock,
            state: RwLock::new(RotationState {
                current_token: initial_token,
                previous_token: None,
                previous_retired_at_ms: None,
                last_rotation_ms: now,
            }),
            rotation_interval_ms: DEFAULT_ROTATION_INTERVAL_MS,
            overlap_ms: DEFAULT_OVERLAP_MS,
        }
    }

    /// Construct with custom interval + overlap (used by tests that want
    /// to compress time without needing the default 5-minute cadence).
    #[must_use]
    pub fn with_schedule(
        initial_token: String,
        clock: Arc<dyn Clock>,
        rotation_interval_ms: u64,
        overlap_ms: u64,
    ) -> Self {
        let now = clock.monotonic_ms();
        Self {
            clock,
            state: RwLock::new(RotationState {
                current_token: initial_token,
                previous_token: None,
                previous_retired_at_ms: None,
                last_rotation_ms: now,
            }),
            rotation_interval_ms,
            overlap_ms,
        }
    }

    /// Current bearer token.
    #[must_use]
    pub fn current(&self) -> String {
        self.state.read().current_token.clone()
    }

    /// Previous token — only returned while still inside the overlap
    /// window.
    #[must_use]
    pub fn previous(&self) -> Option<String> {
        let state = self.state.read();
        let retired = state.previous_retired_at_ms?;
        let token = state.previous_token.as_ref()?;
        let now = self.clock.monotonic_ms();
        if now.saturating_sub(retired) < self.overlap_ms {
            Some(token.clone())
        } else {
            None
        }
    }

    /// Last rotation timestamp on the rotator's clock.
    #[must_use]
    pub fn last_rotation_ms(&self) -> u64 {
        self.state.read().last_rotation_ms
    }

    /// Configured rotation interval in milliseconds.
    #[must_use]
    pub fn rotation_interval_ms(&self) -> u64 {
        self.rotation_interval_ms
    }

    /// Configured overlap window in milliseconds.
    #[must_use]
    pub fn overlap_ms(&self) -> u64 {
        self.overlap_ms
    }

    /// `true` iff `token` matches the current token, OR matches the
    /// previous token and the overlap window has not yet elapsed.
    #[must_use]
    pub fn accepts(&self, token: &str) -> bool {
        let state = self.state.read();
        if state.current_token == token {
            return true;
        }
        if let (Some(prev), Some(retired)) = (
            state.previous_token.as_ref(),
            state.previous_retired_at_ms,
        ) {
            if prev == token {
                let now = self.clock.monotonic_ms();
                if now.saturating_sub(retired) < self.overlap_ms {
                    return true;
                }
            }
        }
        false
    }

    /// Force a rotation immediately. Generates a fresh token, demotes
    /// the current token to "previous", records the retirement
    /// timestamp, and returns the [`RotationOutcome`].
    pub fn rotate_now(&self) -> Result<RotationOutcome, TokenGenError> {
        let new_token = generate_token()?;
        let now = self.clock.monotonic_ms();
        let mut state = self.state.write();
        let retired = std::mem::replace(&mut state.current_token, new_token.clone());
        state.previous_token = Some(retired.clone());
        state.previous_retired_at_ms = Some(now);
        state.last_rotation_ms = now;
        Ok(RotationOutcome {
            new_token,
            retired_token: retired,
            at_ms: now,
        })
    }
}

/// Spawn the production rotation loop. The returned [`JoinHandle`]
/// belongs to the caller — abort it on shutdown to stop the loop.
///
/// The callback runs on the same tokio task as the rotator and is
/// invoked synchronously after each rotation completes. The host wires
/// it to (a) write the new token into the vault, (b) emit the
/// `token_rotated` Tauri event so the webview re-attaches its bearer.
pub fn spawn_rotation_loop<F>(rotator: Arc<TokenRotator>, on_rotate: F) -> JoinHandle<()>
where
    F: Fn(RotationOutcome) + Send + 'static,
{
    let interval = Duration::from_millis(rotator.rotation_interval_ms);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match rotator.rotate_now() {
                Ok(outcome) => on_rotate(outcome),
                Err(err) => {
                    tracing::error!(
                        target: "pellucid::token_rotation",
                        "rotation failed: {err}"
                    );
                }
            }
        }
    })
}

/// Convenience wrapper so `TokenRotator` implements a stable Display
/// without leaking the actual token bytes.
impl fmt::Display for TokenRotator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.read();
        write!(
            f,
            "TokenRotator{{ has_previous = {}, last_rotation_ms = {} }}",
            state.previous_token.is_some(),
            state.last_rotation_ms
        )
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn rotator(initial: &str) -> (Arc<TokenRotator>, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new());
        let r = Arc::new(TokenRotator::with_schedule(
            initial.to_string(),
            clock.clone() as Arc<dyn Clock>,
            DEFAULT_ROTATION_INTERVAL_MS,
            DEFAULT_OVERLAP_MS,
        ));
        (r, clock)
    }

    #[test]
    fn generate_token_is_64_lowercase_hex_chars() {
        let t = generate_token().unwrap();
        assert_eq!(t.len(), 64, "32 bytes => 64 hex chars");
        assert!(
            t.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "token must be lowercase hex, got {t}"
        );
    }

    #[test]
    fn generate_token_does_not_repeat_in_a_thousand_calls() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        for _ in 0..1_000 {
            let t = generate_token().unwrap();
            assert!(set.insert(t), "duplicate token from getrandom (impossible);");
        }
    }

    #[test]
    fn current_and_accepts_initial_token() {
        let (r, _) = rotator("seed-token");
        assert_eq!(r.current(), "seed-token");
        assert!(r.accepts("seed-token"));
        assert!(!r.accepts("other"));
    }

    #[test]
    fn previous_is_none_until_first_rotation() {
        let (r, _) = rotator("seed");
        assert_eq!(r.previous(), None);
    }

    #[test]
    fn rotate_now_changes_current_and_records_previous() {
        let (r, _) = rotator("seed");
        let out = r.rotate_now().unwrap();
        assert_eq!(out.retired_token, "seed");
        assert_eq!(out.new_token, r.current());
        assert_ne!(out.new_token, "seed", "rotation must produce a fresh token");
        assert_eq!(r.previous().as_deref(), Some("seed"));
    }

    #[test]
    fn previous_disappears_after_overlap_elapses() {
        let (r, clock) = rotator("seed");
        r.rotate_now().unwrap();
        clock.advance(DEFAULT_OVERLAP_MS - 1);
        assert!(r.previous().is_some(), "still inside overlap");
        clock.advance(2);
        assert!(
            r.previous().is_none(),
            "must drop previous once overlap elapses"
        );
    }

    #[test]
    fn accepts_returns_true_for_previous_token_inside_overlap() {
        let (r, clock) = rotator("v1");
        r.rotate_now().unwrap();
        let v2 = r.current();
        clock.advance(15_000); // 15s in
        assert!(r.accepts(&v2), "current still works");
        assert!(r.accepts("v1"), "previous still works inside overlap");
        assert!(!r.accepts("v0"), "older tokens rejected");
    }

    #[test]
    fn accepts_returns_false_for_previous_token_outside_overlap() {
        let (r, clock) = rotator("v1");
        r.rotate_now().unwrap();
        let v2 = r.current();
        clock.advance(DEFAULT_OVERLAP_MS + 500);
        assert!(r.accepts(&v2), "current still works after overlap");
        assert!(!r.accepts("v1"), "previous must be rejected after overlap");
    }

    #[test]
    fn second_rotation_demotes_v2_so_v1_is_no_longer_accepted_even_inside_old_overlap() {
        let (r, clock) = rotator("v1");
        r.rotate_now().unwrap();
        let v2 = r.current();
        clock.advance(10_000); // 10s
        r.rotate_now().unwrap();
        let v3 = r.current();
        // v3 is current, v2 is previous, v1 is gone.
        assert!(r.accepts(&v3));
        assert!(r.accepts(&v2), "v2 is now the previous token");
        assert!(!r.accepts("v1"), "v1 was displaced by v2");
    }

    #[test]
    fn last_rotation_ms_advances_with_clock() {
        let (r, clock) = rotator("seed");
        clock.advance(1_234);
        r.rotate_now().unwrap();
        assert_eq!(r.last_rotation_ms(), 1_234);
        clock.advance(500);
        r.rotate_now().unwrap();
        assert_eq!(r.last_rotation_ms(), 1_734);
    }

    #[test]
    fn schedule_constants_match_spec() {
        assert_eq!(DEFAULT_ROTATION_INTERVAL_MS, 300_000);
        assert_eq!(DEFAULT_OVERLAP_MS, 30_000);
        assert_eq!(TOKEN_BYTES, 32);
    }

    #[test]
    fn manual_clock_advances_monotonically() {
        let c = ManualClock::new();
        assert_eq!(c.monotonic_ms(), 0);
        assert_eq!(c.advance(10), 10);
        assert_eq!(c.advance(5), 15);
        assert_eq!(c.monotonic_ms(), 15);
    }

    #[test]
    fn system_clock_returns_increasing_values() {
        let c = SystemClock::new();
        let a = c.monotonic_ms();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = c.monotonic_ms();
        assert!(b >= a, "monotonic must be non-decreasing, got {a} then {b}");
    }

    #[test]
    fn display_does_not_leak_token_value() {
        let (r, _) = rotator("super-secret-token-do-not-print");
        let printed = format!("{}", *r);
        assert!(
            !printed.contains("super-secret-token"),
            "Display must not include the token value: {printed}"
        );
    }

    #[tokio::test]
    async fn spawn_rotation_loop_invokes_callback_after_interval_elapses() {
        let clock = Arc::new(ManualClock::new());
        let r = Arc::new(TokenRotator::with_schedule(
            "init".to_string(),
            clock.clone() as Arc<dyn Clock>,
            // tiny interval so the tokio sleep returns quickly under
            // the test runner.
            5,
            DEFAULT_OVERLAP_MS,
        ));
        let received: Arc<RwLock<Vec<RotationOutcome>>> =
            Arc::new(RwLock::new(Vec::new()));
        let recv2 = received.clone();
        let handle = spawn_rotation_loop(r.clone(), move |out| {
            recv2.write().push(out);
        });
        // Wait for at least two rotations to land.
        for _ in 0..200 {
            if received.read().len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        handle.abort();
        let log = received.read().clone();
        assert!(
            log.len() >= 2,
            "expected ≥ 2 rotations, observed {}",
            log.len()
        );
        // Rotated tokens must all be unique.
        let mut tokens: Vec<&str> =
            log.iter().map(|o| o.new_token.as_str()).collect();
        tokens.sort_unstable();
        let pre = tokens.len();
        tokens.dedup();
        assert_eq!(tokens.len(), pre, "rotation produced duplicate token");
    }
}
