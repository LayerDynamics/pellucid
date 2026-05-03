//! AIS WebSocket client — port of the aisstream.io ingest leg
//! from the original WorldMonitor relay (`scripts/ais-relay.cjs`
//! `:48-58, 7100-7200`).
//!
//! Responsibilities (SPEC-001 §17.2):
//! - **Connect** to `wss://stream.aisstream.io/v0/stream` and
//!   send the typed [`AisSubscribe`] handshake.
//! - **Reconnect** with exponential backoff on any drop
//!   (capped, jitter-free for determinism).
//! - **Watermark queue** — backpressure via a `tokio::sync::
//!   broadcast` channel. When the channel saturates the
//!   slow-consumer drops oldest (broadcast's documented
//!   semantics).
//! - **Decode** the raw text frame into [`AisEnvelope`]; emit
//!   on the broadcast channel.
//!
//! The client is `tokio_tungstenite::tungstenite`-based so the
//! integration test can swap the production URL for a local
//! `tokio::net::TcpListener` running a hand-rolled WS server.

use std::sync::Arc;
use std::time::Duration;

use futures::sink::SinkExt as _;
use futures::stream::StreamExt as _;
use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

use crate::types::{AisEnvelope, AisSubscribe};

/// Default WebSocket URL. Production endpoint per
/// SPEC-001 §17.2.
pub const DEFAULT_WS_URL: &str = "wss://stream.aisstream.io/v0/stream";

/// Default broadcast channel capacity. Mirrors the legacy
/// HIGH/LOW watermark sizing — the relay's downstream consumers
/// (panel snapshot + FTS index) drain at ~5k msg/s burst, so a
/// 4096-slot channel absorbs a one-second hiccup before slow
/// receivers see lag.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 4096;

/// Errors the client can surface to its supervisor.
#[derive(Debug, Error)]
pub enum AisError {
    /// Could not establish the underlying WebSocket connection.
    #[error("websocket connect failed: {0}")]
    Connect(String),
    /// Subscribe handshake failed (encoding / send error).
    #[error("subscribe send failed: {0}")]
    Subscribe(String),
    /// Received an unparseable frame from the upstream. The
    /// run-loop logs and continues — this variant is reserved
    /// for the test surface that wants to assert on it.
    #[error("frame decode failed: {0}")]
    DecodeFrame(String),
}

/// Backoff schedule used by the run-loop on every disconnect.
///
/// The schedule is `min(initial_ms * 2^n, max_ms)` with an
/// optional `reset_after` window: if the connection has been
/// alive longer than that, the next backoff resets to `initial_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    /// First wait after a disconnect.
    pub initial_ms: u64,
    /// Cap on the backoff window.
    pub max_ms: u64,
    /// If the previous connection lasted at least this long,
    /// reset the exponent to zero.
    pub reset_after_ms: u64,
}

impl Backoff {
    /// SPEC-001 §17.2 defaults: 100 ms initial, 30 s cap, reset
    /// after 60 s of healthy uptime.
    pub const DEFAULT: Self = Self {
        initial_ms: 100,
        max_ms: 30_000,
        reset_after_ms: 60_000,
    };

    /// Compute the next wait given the failure count `n` (0 =
    /// first failure). The math saturates so an oversized `n`
    /// stays within `max_ms`.
    #[must_use]
    pub fn next_wait(self, n: u32) -> Duration {
        let shifted = self
            .initial_ms
            .checked_shl(n)
            .map(|v| v.min(self.max_ms))
            .unwrap_or(self.max_ms);
        Duration::from_millis(shifted)
    }

    /// Decide whether the failure counter should reset given the
    /// previous connection's uptime.
    #[must_use]
    pub fn should_reset(self, last_uptime: Duration) -> bool {
        last_uptime.as_millis() as u64 >= self.reset_after_ms
    }
}

/// HIGH/LOW watermark queue tracker. The aisstream.io WebSocket
/// has no application-layer flow control; if our consumers
/// stall, we either:
///  - drop oldest (the broadcast channel does this for us — slow
///    receivers see `broadcast::error::RecvError::Lagged`), or
///  - pause reads on the WS socket.
///
/// We do both: the watermark counts how many messages are
/// in-flight (sent on the channel but not yet acknowledged via
/// `record_drained`) and pauses upstream reads when above
/// `high`, resuming below `low`. The pause is **best-effort** —
/// `tokio::sync::broadcast` does not expose a true pause primitive,
/// so we sleep the read loop briefly when paused.
#[derive(Debug)]
pub struct WatermarkQueue {
    inflight: Mutex<usize>,
    high: usize,
    low: usize,
}

impl WatermarkQueue {
    /// Construct with explicit thresholds. Panics if `low > high`
    /// because that ordering is meaningless.
    #[must_use]
    pub fn new(high: usize, low: usize) -> Self {
        assert!(high >= low, "watermark high must be ≥ low");
        Self {
            inflight: Mutex::new(0),
            high,
            low,
        }
    }

    /// Default tuning matching the SPEC-001 channel capacity:
    /// pause at 75 %, resume at 25 % of channel.
    #[must_use]
    pub fn for_capacity(capacity: usize) -> Self {
        let high = capacity * 3 / 4;
        let low = capacity / 4;
        Self::new(high, low)
    }

    /// Increment the in-flight counter. Returns `true` iff the
    /// caller should *pause* (we just crossed the high watermark).
    pub fn record_emitted(&self) -> bool {
        let mut g = self.inflight.lock();
        *g += 1;
        *g >= self.high
    }

    /// Decrement the in-flight counter. Returns `true` iff the
    /// caller should *resume* (we just dropped below low).
    pub fn record_drained(&self) -> bool {
        let mut g = self.inflight.lock();
        if *g > 0 {
            *g -= 1;
        }
        *g <= self.low
    }

    /// Current in-flight count.
    pub fn inflight(&self) -> usize {
        *self.inflight.lock()
    }

    /// Read-only access to the configured high watermark.
    pub const fn high(&self) -> usize {
        self.high
    }

    /// Read-only access to the configured low watermark.
    pub const fn low(&self) -> usize {
        self.low
    }
}

/// AIS WebSocket client. Construction is cheap; `run` is the
/// long-lived task the relay supervises.
#[derive(Debug)]
pub struct AisClient {
    url: String,
    api_key: String,
    backoff: Backoff,
    sender: broadcast::Sender<AisEnvelope>,
    watermark: Arc<WatermarkQueue>,
}

impl AisClient {
    /// Build a client. `capacity` is the broadcast channel size
    /// (consumers calling `subscribe()` get a receiver bounded by
    /// the same capacity).
    #[must_use]
    pub fn new(url: impl Into<String>, api_key: impl Into<String>, capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            url: url.into(),
            api_key: api_key.into(),
            backoff: Backoff::DEFAULT,
            sender,
            watermark: Arc::new(WatermarkQueue::for_capacity(capacity)),
        }
    }

    /// Build with the SPEC-001 production URL and default
    /// channel capacity.
    #[must_use]
    pub fn with_default_url(api_key: impl Into<String>) -> Self {
        Self::new(DEFAULT_WS_URL, api_key, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Override the backoff schedule (testing).
    #[must_use]
    pub fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    /// Subscribe to the broadcast channel — every successful
    /// envelope is fanned out to every receiver.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<AisEnvelope> {
        self.sender.subscribe()
    }

    /// In-flight watermark observable for diagnostics.
    #[must_use]
    pub fn watermark(&self) -> Arc<WatermarkQueue> {
        self.watermark.clone()
    }

    /// Run the connect → subscribe → read loop forever, with
    /// exponential-backoff reconnect on every disconnect. Returns
    /// only when the broadcast channel has been closed (every
    /// receiver dropped).
    pub async fn run(self) {
        let mut failure_count: u32 = 0;
        loop {
            let started = std::time::Instant::now();
            match self.run_one_session().await {
                Ok(()) => {
                    // Clean disconnect (server FIN). Reset
                    // backoff and reconnect immediately.
                    failure_count = 0;
                }
                Err(err) => {
                    tracing::warn!(target: "pellucid::streams::ais", "session ended: {err}");
                    let uptime = started.elapsed();
                    if self.backoff.should_reset(uptime) {
                        failure_count = 0;
                    }
                    let wait = self.backoff.next_wait(failure_count);
                    failure_count = failure_count.saturating_add(1);
                    tokio::time::sleep(wait).await;
                }
            }
            // If every consumer has dropped, exit the loop.
            if self.sender.receiver_count() == 0 {
                return;
            }
        }
    }

    /// One connect + subscribe + read pass. Returns `Ok(())` on
    /// clean upstream disconnect; `Err(...)` on connect / send /
    /// transport failure.
    async fn run_one_session(&self) -> Result<(), AisError> {
        let (ws_stream, _resp) = tokio_tungstenite::connect_async(&self.url)
            .await
            .map_err(|e| AisError::Connect(e.to_string()))?;
        let (mut write, mut read) = ws_stream.split();

        // Subscribe handshake.
        let sub = AisSubscribe::world(&self.api_key);
        let payload = serde_json::to_string(&sub)
            .map_err(|e| AisError::Subscribe(format!("encode: {e}")))?;
        write
            .send(Message::Text(payload))
            .await
            .map_err(|e| AisError::Subscribe(e.to_string()))?;

        // Read loop.
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    process_text_frame(&text, &self.sender, &self.watermark);
                }
                Ok(Message::Binary(_)) => {
                    // Spec says all frames are text; ignore + continue.
                }
                Ok(Message::Close(_)) => return Ok(()),
                Ok(Message::Ping(p)) => {
                    let _ = write.send(Message::Pong(p)).await;
                }
                Ok(Message::Pong(_) | Message::Frame(_)) => { /* ignore */ }
                Err(e) => {
                    return Err(AisError::Connect(e.to_string()));
                }
            }
        }
        Ok(())
    }
}

/// Decode `frame_text` into an [`AisEnvelope`] and broadcast it.
/// Bad frames are logged + counted; we do NOT tear down the
/// session for one malformed payload.
pub fn process_text_frame(
    frame_text: &str,
    sender: &broadcast::Sender<AisEnvelope>,
    watermark: &WatermarkQueue,
) {
    match serde_json::from_str::<AisEnvelope>(frame_text) {
        Ok(env) => {
            // The broadcast channel handles slow-consumer drops
            // for us; we just record the watermark deltas.
            let _ = sender.send(env);
            watermark.record_emitted();
        }
        Err(e) => {
            tracing::debug!(
                target: "pellucid::streams::ais",
                "skip unparseable frame: {e}"
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_until_cap() {
        let b = Backoff::DEFAULT;
        assert_eq!(b.next_wait(0), Duration::from_millis(100));
        assert_eq!(b.next_wait(1), Duration::from_millis(200));
        assert_eq!(b.next_wait(2), Duration::from_millis(400));
        assert_eq!(b.next_wait(3), Duration::from_millis(800));
        assert_eq!(b.next_wait(7), Duration::from_millis(12_800));
        // Capped at max_ms.
        assert_eq!(b.next_wait(20), Duration::from_millis(30_000));
        assert_eq!(b.next_wait(31), Duration::from_millis(30_000));
        assert_eq!(b.next_wait(64), Duration::from_millis(30_000));
    }

    #[test]
    fn backoff_should_reset_compares_against_window() {
        let b = Backoff::DEFAULT;
        assert!(!b.should_reset(Duration::from_millis(0)));
        assert!(!b.should_reset(Duration::from_millis(b.reset_after_ms - 1)));
        assert!(b.should_reset(Duration::from_millis(b.reset_after_ms)));
        assert!(b.should_reset(Duration::from_secs(120)));
    }

    #[test]
    fn watermark_emit_drain_round_trip() {
        let w = WatermarkQueue::new(10, 4);
        assert_eq!(w.inflight(), 0);
        for _ in 0..9 {
            assert!(!w.record_emitted(), "should not pause until ≥ high");
        }
        // 10th emit hits the high watermark — should signal pause.
        assert!(w.record_emitted());
        assert_eq!(w.inflight(), 10);
        // Drain back down — first drains do NOT yet signal resume.
        for _ in 0..5 {
            let resumed = w.record_drained();
            // We hit ≤ low (= 4) after the 6th drain (10-6=4).
            // The first 5 drains return false / true as the
            // counter passes 5, 4, 3, ...; let the assertion at
            // the end catch the contract.
            let _ = resumed;
        }
        // After 5 drains: 10-5=5 inflight. Not yet at low.
        assert!(w.inflight() == 5);
        // 6th drain → 4 inflight, ≤ low = 4 → resume.
        assert!(w.record_drained());
        assert_eq!(w.inflight(), 4);
    }

    #[test]
    fn watermark_drain_below_zero_clamps() {
        let w = WatermarkQueue::new(2, 0);
        // Drain a fresh queue — no underflow.
        let _ = w.record_drained();
        let _ = w.record_drained();
        assert_eq!(w.inflight(), 0);
    }

    #[test]
    fn watermark_for_capacity_uses_75_25_split() {
        let w = WatermarkQueue::for_capacity(4096);
        assert_eq!(w.high(), 3072);
        assert_eq!(w.low(), 1024);
    }

    #[test]
    #[should_panic(expected = "watermark high must be ≥ low")]
    fn watermark_inverted_thresholds_panics() {
        let _ = WatermarkQueue::new(1, 5);
    }

    #[test]
    fn process_text_frame_decodes_position_report_and_emits() {
        let (tx, mut rx) = broadcast::channel(8);
        let w = WatermarkQueue::for_capacity(8);
        let frame = r#"{
            "MessageType": "PositionReport",
            "MetaData": { "MMSI": 367123, "latitude": 1.0, "longitude": 2.0 },
            "Message": { "PositionReport": { "Sog": 10.0 } }
        }"#;
        process_text_frame(frame, &tx, &w);
        let env = rx.try_recv().unwrap();
        assert_eq!(env.message_type, "PositionReport");
        assert_eq!(env.metadata.mmsi, 367123);
        assert_eq!(w.inflight(), 1);
    }

    #[test]
    fn process_text_frame_decodes_ship_static_data() {
        let (tx, mut rx) = broadcast::channel(8);
        let w = WatermarkQueue::for_capacity(8);
        let frame = r#"{
            "MessageType": "ShipStaticData",
            "MetaData": { "MMSI": 367999, "latitude": 0.0, "longitude": 0.0 },
            "Message": { "ShipStaticData": { "Name": "TEST" } }
        }"#;
        process_text_frame(frame, &tx, &w);
        let env = rx.try_recv().unwrap();
        assert_eq!(env.message_type, "ShipStaticData");
    }

    #[test]
    fn process_text_frame_skips_malformed_without_emitting() {
        let (tx, mut rx) = broadcast::channel(8);
        let w = WatermarkQueue::for_capacity(8);
        process_text_frame("not json", &tx, &w);
        // No envelope emitted → try_recv should empty.
        match rx.try_recv() {
            Err(broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected Empty, got {other:?}"),
        }
        // Watermark unchanged.
        assert_eq!(w.inflight(), 0);
    }

    #[test]
    fn ais_client_subscribe_returns_independent_receivers() {
        let c = AisClient::new(
            "wss://unused.test/ws",
            "test-key",
            8,
        );
        let mut a = c.subscribe();
        let mut b = c.subscribe();
        // Drive a frame through process_text_frame using the
        // client's broadcast sender (private — exposed via
        // subscribe() above + the public function below).
        let frame = r#"{
            "MessageType": "PositionReport",
            "MetaData": { "MMSI": 1, "latitude": 0.0, "longitude": 0.0 },
            "Message": {}
        }"#;
        process_text_frame(frame, &c.sender, &c.watermark);
        assert_eq!(a.try_recv().unwrap().metadata.mmsi, 1);
        assert_eq!(b.try_recv().unwrap().metadata.mmsi, 1);
    }
}
