//! Shared in-memory accumulator over the AIS WebSocket stream.
//!
//! The [`AisClient`] in `crate::ais` emits a continuous
//! broadcast of `AisEnvelope` rows. AIS data is real-time
//! only — there is no free polling REST API for global vessel
//! positions — so any "snapshot" view (chokepoint counts,
//! per-region density) has to maintain state in-process.
//!
//! [`MaritimeState`] is that state. Production wires:
//!
//! 1. `AisClient::run_loop` emits `AisEnvelope` rows on a
//!    `broadcast::Sender<AisEnvelope>`.
//! 2. [`MaritimeState::spawn_consumer`] subscribes to the
//!    broadcast and updates a sliding `MMSI → VesselFix`
//!    map.
//! 3. Maritime seeders read [`MaritimeState::snapshot`] /
//!    [`MaritimeState::vessels_in_bbox`] on their cycle.
//!
//! The state is bounded: each MMSI's most-recent fix replaces
//! the prior one; entries older than `ttl` are pruned by the
//! background tick.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use tokio::sync::broadcast::{error::RecvError, Receiver};
use tokio::task::JoinHandle;

use crate::types::AisEnvelope;

/// `(lamin, lomin, lamax, lomax)` bounding box.
pub type BBox = (f64, f64, f64, f64);

/// `(name, BBox)` named region.
pub type NamedBBox<'a> = (&'a str, BBox);

/// Default fix TTL — 15 minutes. Vessels reporting AIS more
/// than 15 minutes ago are considered stale and dropped from
/// the snapshot.
pub const DEFAULT_FIX_TTL: Duration = Duration::from_secs(15 * 60);

/// Default prune cadence — every 60 s.
pub const DEFAULT_PRUNE_INTERVAL: Duration = Duration::from_secs(60);

/// One MMSI's most-recent AIS fix.
#[derive(Clone, Debug)]
pub struct VesselFix {
    /// MMSI (Maritime Mobile Service Identity).
    pub mmsi: u32,
    /// WGS84 latitude.
    pub latitude: f64,
    /// WGS84 longitude.
    pub longitude: f64,
    /// AIS message-type slug (`PositionReport`, `ShipStaticData`, …).
    pub message_type: String,
    /// Wall-clock instant we first observed this fix.
    pub seen_at: Instant,
}

impl VesselFix {
    /// `true` iff `(lat, lon)` is inside `(lamin, lomin, lamax, lomax)`.
    #[must_use]
    pub fn is_in_bbox(&self, bbox: BBox) -> bool {
        let (lamin, lomin, lamax, lomax) = bbox;
        self.latitude >= lamin
            && self.latitude <= lamax
            && self.longitude >= lomin
            && self.longitude <= lomax
    }
}

/// Sliding-window map of `MMSI → most-recent VesselFix`.
#[derive(Clone, Debug, Default)]
pub struct MaritimeState {
    inner: Arc<RwLock<HashMap<u32, VesselFix>>>,
}

impl MaritimeState {
    /// Build an empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one AIS envelope to the state. The most-recent
    /// fix per MMSI wins; envelopes without lat/lon are
    /// ignored.
    pub fn apply(&self, env: &AisEnvelope) {
        let Some(fix) = vessel_fix_from_envelope(env) else {
            return;
        };
        self.inner.write().insert(fix.mmsi, fix);
    }

    /// Drop fixes older than `ttl` from the state. Returns the
    /// number of pruned entries.
    pub fn prune(&self, ttl: Duration) -> usize {
        let cutoff = Instant::now() - ttl;
        let mut guard = self.inner.write();
        let before = guard.len();
        guard.retain(|_mmsi, fix| fix.seen_at >= cutoff);
        before - guard.len()
    }

    /// Snapshot: total vessel count.
    #[must_use]
    pub fn vessel_count(&self) -> usize {
        self.inner.read().len()
    }

    /// Snapshot: vessel count inside `bbox` (inclusive).
    #[must_use]
    pub fn vessels_in_bbox(&self, bbox: BBox) -> usize {
        self.inner
            .read()
            .values()
            .filter(|f| f.is_in_bbox(bbox))
            .count()
    }

    /// Snapshot: per-region vessel counts. Returns one entry
    /// per `(name, bbox)` pair in input order.
    #[must_use]
    pub fn vessels_by_region(&self, regions: &[NamedBBox]) -> Vec<(String, usize)> {
        let guard = self.inner.read();
        regions
            .iter()
            .map(|(name, bbox)| {
                let count = guard.values().filter(|f| f.is_in_bbox(*bbox)).count();
                ((*name).to_string(), count)
            })
            .collect()
    }

    /// Spawn a consumer that drains the supplied broadcast
    /// receiver into the state and prunes stale fixes on a
    /// periodic tick. Returns the join handle for cooperative
    /// shutdown.
    #[must_use]
    pub fn spawn_consumer(
        &self,
        mut rx: Receiver<AisEnvelope>,
        ttl: Duration,
        prune_interval: Duration,
    ) -> JoinHandle<()> {
        let state = self.clone();
        tokio::spawn(async move {
            let mut prune_ticker = tokio::time::interval(prune_interval);
            // First tick fires immediately — skip it so the
            // initial prune happens after `prune_interval`.
            prune_ticker.tick().await;
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Ok(env) => state.apply(&env),
                            Err(RecvError::Lagged(_)) => continue,
                            Err(RecvError::Closed) => break,
                        }
                    }
                    _ = prune_ticker.tick() => {
                        let _ = state.prune(ttl);
                    }
                }
            }
        })
    }
}

fn vessel_fix_from_envelope(env: &AisEnvelope) -> Option<VesselFix> {
    // The aisstream.io v0 envelope deserialises with default
    // f64 = 0.0 for missing lat/lon. Treat (0, 0) as "no fix"
    // — a real vessel at the geographic null island is
    // vanishingly improbable.
    let lat = env.metadata.latitude;
    let lon = env.metadata.longitude;
    if lat == 0.0 && lon == 0.0 {
        return None;
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some(VesselFix {
        mmsi: env.metadata.mmsi,
        latitude: lat,
        longitude: lon,
        message_type: env.message_type.clone(),
        seen_at: Instant::now(),
    })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::AisMetadata;
    use tokio::sync::broadcast;

    fn env(mmsi: u32, lat: f64, lon: f64) -> AisEnvelope {
        AisEnvelope {
            metadata: AisMetadata {
                mmsi,
                latitude: lat,
                longitude: lon,
                ship_name: Some(format!("MV-{mmsi}")),
                time_utc: Some("2026-05-04T12:00:00Z".into()),
            },
            message: serde_json::json!({}),
            message_type: "PositionReport".into(),
        }
    }

    fn env_no_position(mmsi: u32) -> AisEnvelope {
        AisEnvelope {
            metadata: AisMetadata {
                mmsi,
                latitude: 0.0,
                longitude: 0.0,
                ship_name: None,
                time_utc: None,
            },
            message: serde_json::json!({}),
            message_type: "ShipStaticData".into(),
        }
    }

    #[test]
    fn apply_adds_new_mmsi() {
        let state = MaritimeState::new();
        state.apply(&env(123_456_789, 35.0, 38.0));
        assert_eq!(state.vessel_count(), 1);
    }

    #[test]
    fn apply_replaces_prior_fix_for_same_mmsi() {
        let state = MaritimeState::new();
        state.apply(&env(1, 0.0, 0.0));
        state.apply(&env(1, 10.0, 20.0));
        assert_eq!(state.vessel_count(), 1);
        assert_eq!(state.vessels_in_bbox((9.0, 19.0, 11.0, 21.0)), 1);
        assert_eq!(state.vessels_in_bbox((-1.0, -1.0, 1.0, 1.0)), 0);
    }

    #[test]
    fn apply_skips_envelopes_without_position() {
        let state = MaritimeState::new();
        state.apply(&env_no_position(1));
        assert_eq!(state.vessel_count(), 0);
    }

    #[test]
    fn vessels_in_bbox_counts_inclusively() {
        let state = MaritimeState::new();
        state.apply(&env(1, 35.0, 38.0)); // inside
        state.apply(&env(2, 30.0, 30.0)); // outside
        state.apply(&env(3, 40.0, 40.0)); // on boundary
        assert_eq!(state.vessels_in_bbox((35.0, 38.0, 40.0, 40.0)), 2);
    }

    #[test]
    fn vessels_by_region_returns_per_bbox_counts() {
        let state = MaritimeState::new();
        state.apply(&env(1, 35.0, 38.0));
        state.apply(&env(2, -5.0, 100.0));
        let regions = vec![
            ("med", (30.0_f64, 30.0_f64, 40.0_f64, 40.0_f64)),
            ("malacca", (-10.0_f64, 95.0_f64, 5.0_f64, 105.0_f64)),
            ("none", (60.0_f64, -150.0_f64, 70.0_f64, -140.0_f64)),
        ];
        let counts = state.vessels_by_region(&regions);
        assert_eq!(
            counts,
            vec![
                ("med".to_string(), 1),
                ("malacca".to_string(), 1),
                ("none".to_string(), 0),
            ]
        );
    }

    #[test]
    fn prune_drops_stale_fixes() {
        let state = MaritimeState::new();
        state.apply(&env(1, 35.0, 38.0));
        std::thread::sleep(Duration::from_millis(20));
        let pruned = state.prune(Duration::from_millis(10));
        assert_eq!(pruned, 1);
        assert_eq!(state.vessel_count(), 0);
    }

    #[tokio::test]
    async fn spawn_consumer_applies_envelopes_from_broadcast() {
        let (tx, rx) = broadcast::channel::<AisEnvelope>(8);
        let state = MaritimeState::new();
        let handle = state.spawn_consumer(rx, Duration::from_secs(60), Duration::from_secs(60));
        tx.send(env(11, 35.0, 38.0)).unwrap();
        tx.send(env(12, -5.0, 100.0)).unwrap();
        // Give the consumer a moment to drain.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(state.vessel_count(), 2);
        drop(tx);
        let _ = handle.await;
    }

    #[tokio::test]
    async fn spawn_consumer_exits_when_sender_drops() {
        let (tx, rx) = broadcast::channel::<AisEnvelope>(8);
        let state = MaritimeState::new();
        let handle = state.spawn_consumer(rx, Duration::from_secs(60), Duration::from_secs(60));
        drop(tx);
        // Should resolve cleanly within a reasonable window.
        tokio::time::timeout(Duration::from_millis(500), handle)
            .await
            .expect("consumer task exited")
            .expect("no join error");
    }

    #[test]
    fn vessel_fix_is_in_bbox_handles_meridian_boundary() {
        let f = VesselFix {
            mmsi: 1,
            latitude: 0.0,
            longitude: -179.0,
            message_type: "PositionReport".into(),
            seen_at: Instant::now(),
        };
        // Western Pacific bbox doesn't include -179.
        assert!(!f.is_in_bbox((-10.0, 100.0, 50.0, 160.0)));
        // North-America bbox does.
        assert!(f.is_in_bbox((-10.0, -180.0, 50.0, -150.0)));
    }
}
