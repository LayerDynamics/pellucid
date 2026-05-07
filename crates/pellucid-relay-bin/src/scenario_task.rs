//! Scenario worker background task — wraps `pellucid_workers::scenario`
//! into the same shutdown-coordinated shape as `ais_task`.
//!
//! Spawn behaviour:
//!
//! - When `UPSTASH_REDIS_REST_URL` / `UPSTASH_REDIS_REST_TOKEN` are
//!   present, [`spawn`] starts the worker and returns a handle.
//! - When they're absent, [`spawn`] returns `None` and logs a single
//!   `info!`. The relay continues without scenario processing — dev
//!   mode parity with `ais_task::spawn`.
//!
//! `deep_forecast` and `simulation` are **not** auto-spawned by the
//! relay because their compute requires a [`pellucid_workers::deep_forecast::DeepForecastDriver`]
//! / [`pellucid_workers::simulation::SimulationDriver`] implementation
//! that lives close to the LLM stack. A future `pellucid-forecast-bin`
//! (or the relay binary, after the LLM stack is wired) will own those
//! drivers; this crate spawns the queue plumbing only.

use std::time::Duration;

use tokio::task::JoinHandle;
use tracing::{info, warn};

use pellucid_workers::redis::RedisClient;
use pellucid_workers::scenario;

/// Handle for cooperative shutdown.
#[derive(Debug)]
pub struct ScenarioHandle {
    /// The worker loop task.
    pub task: JoinHandle<()>,
}

/// Spawn the scenario worker if Upstash credentials are present in the
/// process environment. Returns `None` otherwise.
#[must_use]
pub fn spawn() -> Option<ScenarioHandle> {
    let redis = match RedisClient::from_env() {
        Ok(r) => r,
        Err(err) => {
            info!(
                target: "pellucid::relay::scenario",
                error = %err,
                "scenario worker not started — Upstash credentials absent"
            );
            return None;
        }
    };
    let task = tokio::spawn(async move {
        info!(
            target: "pellucid::relay::scenario",
            base_url = redis.base_url(),
            "scenario worker starting"
        );
        if let Err(err) = scenario::run_worker(&redis, scenario::WorkerOptions::default()).await {
            warn!(
                target: "pellucid::relay::scenario",
                error = %err,
                "scenario worker exited with error"
            );
        } else {
            info!(target: "pellucid::relay::scenario", "scenario worker exited cleanly");
        }
    });
    Some(ScenarioHandle { task })
}

/// Drain the worker task within `grace`. Returns `true` iff the task
/// exited cleanly.
pub async fn shutdown(handle: ScenarioHandle, grace: Duration) -> bool {
    handle.task.abort();
    tokio::time::timeout(grace, handle.task).await.is_ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_returns_none_without_credentials() {
        // Save + clear the env to ensure the boot decision falls into
        // the "absent" branch.
        let prev_url = std::env::var("UPSTASH_REDIS_REST_URL").ok();
        let prev_tok = std::env::var("UPSTASH_REDIS_REST_TOKEN").ok();
        std::env::remove_var("UPSTASH_REDIS_REST_URL");
        std::env::remove_var("UPSTASH_REDIS_REST_TOKEN");

        assert!(spawn().is_none());

        if let Some(v) = prev_url {
            std::env::set_var("UPSTASH_REDIS_REST_URL", v);
        }
        if let Some(v) = prev_tok {
            std::env::set_var("UPSTASH_REDIS_REST_TOKEN", v);
        }
    }
}
