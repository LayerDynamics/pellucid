//! AIS WebSocket background task.
//!
//! The relay subscribes to aisstream.io's global feed and
//! drains the resulting `AisEnvelope` broadcast into the
//! shared `MaritimeState` accumulator. The accumulator is
//! the source of truth for the maritime + logistics seeders
//! that publish per-region vessel counts.
//!
//! `pellucid_streams::AisClient` owns its broadcast channel
//! (the producer's run-loop is the only writer). We
//! `subscribe()` for the maritime-state consumer task before
//! spawning the producer; the producer exits when every
//! receiver has been dropped.

use std::time::Duration;

use tokio::task::JoinHandle;
use tracing::info;

use pellucid_streams::{
    AisClient, MaritimeState, DEFAULT_CHANNEL_CAPACITY, DEFAULT_FIX_TTL, DEFAULT_PRUNE_INTERVAL,
    DEFAULT_WS_URL,
};

/// Handles for cooperative shutdown.
#[derive(Debug)]
pub struct AisHandles {
    /// Producer task — runs the AIS WebSocket loop.
    pub producer: JoinHandle<()>,
    /// Consumer task — drains broadcast into MaritimeState.
    pub consumer: JoinHandle<()>,
}

/// Spawn the AIS pipeline. Returns handles that the caller
/// must hold for shutdown coordination.
///
/// `state` is shared with the maritime + logistics seeders
/// (they read snapshots; the consumer task writes).
///
/// When `ais_api_key` is `None` the function returns `None`
/// — the relay can boot without AIS in dev mode.
#[must_use]
pub fn spawn(ais_api_key: Option<String>, state: MaritimeState) -> Option<AisHandles> {
    let api_key = ais_api_key?;
    let client = AisClient::new(DEFAULT_WS_URL, api_key, DEFAULT_CHANNEL_CAPACITY);
    let rx = client.subscribe();
    let consumer = state.spawn_consumer(rx, DEFAULT_FIX_TTL, DEFAULT_PRUNE_INTERVAL);
    let producer = tokio::spawn(async move {
        info!("AIS task: connecting to aisstream.io");
        client.run().await;
        info!("AIS task: producer exited");
    });
    Some(AisHandles { producer, consumer })
}

/// Wait up to `grace` for both tasks to finish. Caller must
/// arrange for the producer to exit (typically by dropping
/// every receiver, which `consumer.abort()` triggers).
pub async fn shutdown(handles: AisHandles, grace: Duration) -> bool {
    let AisHandles { producer, consumer } = handles;
    consumer.abort();
    let drain = async {
        let _ = consumer.await;
        let _ = producer.await;
    };
    tokio::time::timeout(grace, drain).await.is_ok()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_streams::types::AisEnvelope;
    use tokio::sync::broadcast;

    #[tokio::test]
    async fn spawn_returns_none_without_api_key() {
        let state = MaritimeState::new();
        assert!(spawn(None, state).is_none());
    }

    #[tokio::test]
    async fn shutdown_drains_within_grace() {
        // Stand up the consumer side directly — the producer
        // is a no-op task standing in for the WebSocket loop.
        let state = MaritimeState::new();
        let (_tx, rx) = broadcast::channel::<AisEnvelope>(8);
        let consumer = state.spawn_consumer(rx, DEFAULT_FIX_TTL, DEFAULT_PRUNE_INTERVAL);
        let producer = tokio::spawn(async {});
        let handles = AisHandles { producer, consumer };
        // 500ms is generous for both abort + cooperative exit.
        let drained = shutdown(handles, Duration::from_millis(500)).await;
        assert!(drained, "tasks must drain within grace");
    }
}
