//! Simulation worker — Rust port of
//! `worldmonitor/scripts/process-simulation-tasks.mjs` + the
//! `processNextSimulationTask` body in `seed-forecasts.mjs`.
//!
//! Same shape as [`crate::deep_forecast`]: this crate owns the queue
//! plumbing (BLMOVE → claim → SETEX result), the actual simulation
//! compute is delegated to a [`SimulationDriver`] supplied by the relay
//! binary (where the LLM / signal stacks already live).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::WorkerError;
use crate::redis::{ListEnd, RedisClient};

/// FIFO pending queue.
pub const QUEUE_KEY: &str = "simulation-queue:pending";
/// In-flight processing list.
pub const PROCESSING_KEY: &str = "simulation-queue:processing";
/// 24 hours.
pub const RESULT_TTL_SECONDS: u64 = 86_400;
/// Idle poll interval — `seed-forecasts.mjs:56` `SIMULATION_POLL_INTERVAL_MS`.
pub const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// `BLMOVE` block timeout (Upstash REST is non-blocking).
pub const BLMOVE_TIMEOUT_SECONDS: u64 = 30;

/// One simulation job. Generic shape — driver decodes payload.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SimulationJob {
    /// Unique job identifier.
    pub job_id: String,
    /// Which simulation type (`"economic"`, `"conflict"`, etc). Echoed
    /// for log filtering.
    #[serde(default)]
    pub kind: Option<String>,
    /// Caller-supplied payload — opaque to this crate.
    #[serde(default)]
    pub payload: Value,
}

/// Driver contract.
#[async_trait]
pub trait SimulationDriver: Send + Sync {
    /// Run the simulation. Return the JSON to write into the result row.
    ///
    /// # Errors
    /// See [`crate::deep_forecast::DeepForecastDriver::compute`] — same
    /// rules apply.
    async fn compute(&self, job: &SimulationJob) -> Result<Value, WorkerError>;
}

/// Worker-loop options.
#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// Run a single iteration and return.
    pub once: bool,
    /// `BLMOVE` block timeout.
    pub blmove_timeout_secs: u64,
    /// Sleep between empty polls.
    pub empty_backoff: Duration,
    /// Result TTL (s).
    pub result_ttl_secs: u64,
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            once: false,
            blmove_timeout_secs: BLMOVE_TIMEOUT_SECONDS,
            empty_backoff: POLL_INTERVAL,
            result_ttl_secs: RESULT_TTL_SECONDS,
        }
    }
}

/// Per-iteration outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IterationOutcome {
    /// Queue empty.
    Idle,
    /// Job processed successfully.
    Done {
        /// Echoed job id.
        job_id: String,
    },
    /// Job dequeued but parse failed — discarded.
    Discarded,
    /// Driver compute failed — `failed` row written.
    Failed {
        /// Echoed job id.
        job_id: String,
    },
    /// Idempotent dedupe — result already existed.
    AlreadyProcessed {
        /// Echoed job id.
        job_id: String,
    },
}

/// Run the simulation worker. Mirrors `runSimulationWorker` in the JS
/// source.
///
/// # Errors
/// Catastrophic transport failures only.
pub async fn run_worker<D: SimulationDriver>(
    redis: &RedisClient,
    driver: &D,
    options: WorkerOptions,
) -> Result<(), WorkerError> {
    requeue_orphaned_jobs(redis).await?;
    loop {
        match poll_once(redis, driver, &options).await {
            Ok(IterationOutcome::Idle) if options.once => return Ok(()),
            Ok(IterationOutcome::Idle) => {
                tokio::time::sleep(options.empty_backoff).await;
            }
            Ok(_) if options.once => return Ok(()),
            Ok(_) => {}
            Err(err) => {
                tracing::error!(
                    target: "pellucid::workers::simulation",
                    error = %err,
                    "simulation poll error"
                );
                if options.once {
                    return Err(err);
                }
                tokio::time::sleep(options.empty_backoff).await;
            }
        }
    }
}

/// Drain `PROCESSING_KEY` back into `QUEUE_KEY` at startup.
///
/// # Errors
/// Transport failures.
pub async fn requeue_orphaned_jobs(redis: &RedisClient) -> Result<(), WorkerError> {
    let mut count = 0u32;
    while redis
        .lmove(PROCESSING_KEY, QUEUE_KEY, ListEnd::Right, ListEnd::Left)
        .await?
        .is_some()
    {
        count += 1;
    }
    if count > 0 {
        tracing::info!(
            target: "pellucid::workers::simulation",
            count,
            "requeued orphaned simulation jobs"
        );
    }
    Ok(())
}

/// One iteration. Public for tests.
///
/// # Errors
/// Transport failures only.
pub async fn poll_once<D: SimulationDriver>(
    redis: &RedisClient,
    driver: &D,
    options: &WorkerOptions,
) -> Result<IterationOutcome, WorkerError> {
    let raw = redis
        .blmove(QUEUE_KEY, PROCESSING_KEY, options.blmove_timeout_secs)
        .await?;
    let Some(raw) = raw else {
        return Ok(IterationOutcome::Idle);
    };

    let job: SimulationJob = match serde_json::from_str(&raw) {
        Ok(j) => j,
        Err(_) => {
            tracing::warn!(
                target: "pellucid::workers::simulation",
                payload = %raw.chars().take(120).collect::<String>(),
                "unparseable simulation job, discarding"
            );
            redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
            return Ok(IterationOutcome::Discarded);
        }
    };

    if job.job_id.is_empty() {
        tracing::warn!(
            target: "pellucid::workers::simulation",
            "simulation job has empty job_id, discarding"
        );
        redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
        return Ok(IterationOutcome::Discarded);
    }

    let result_key = format!("simulation-result:{}", job.job_id);
    if let Ok(Some(_)) = redis.get_json(&result_key).await {
        tracing::info!(
            target: "pellucid::workers::simulation",
            job_id = %job.job_id,
            "simulation already processed, skipping"
        );
        redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
        return Ok(IterationOutcome::AlreadyProcessed { job_id: job.job_id });
    }

    let processing_state =
        json!({ "status": "processing", "startedAt": crate::scenario::now_ms() });
    redis
        .setex(&result_key, options.result_ttl_secs, &processing_state)
        .await
        .ok();

    match driver.compute(&job).await {
        Ok(result) => {
            let payload = json!({
                "status": "done",
                "result": result,
                "completedAt": crate::scenario::now_ms(),
            });
            redis
                .setex(&result_key, options.result_ttl_secs, &payload)
                .await?;
            redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
            tracing::info!(
                target: "pellucid::workers::simulation",
                job_id = %job.job_id,
                "simulation complete"
            );
            Ok(IterationOutcome::Done { job_id: job.job_id })
        }
        Err(err) => {
            tracing::error!(
                target: "pellucid::workers::simulation",
                job_id = %job.job_id,
                error = %err,
                "simulation driver error"
            );
            let payload = json!({
                "status": "failed",
                "error": err.to_string(),
                "failedAt": crate::scenario::now_ms(),
            });
            redis
                .setex(&result_key, options.result_ttl_secs, &payload)
                .await
                .ok();
            redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
            Ok(IterationOutcome::Failed { job_id: job.job_id })
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    struct EchoDriver;
    #[async_trait]
    impl SimulationDriver for EchoDriver {
        async fn compute(&self, job: &SimulationJob) -> Result<Value, WorkerError> {
            Ok(json!({"echoed": job.kind.clone()}))
        }
    }

    #[test]
    fn options_default_values() {
        let opts = WorkerOptions::default();
        assert!(!opts.once);
        assert_eq!(opts.empty_backoff, POLL_INTERVAL);
        assert_eq!(opts.blmove_timeout_secs, BLMOVE_TIMEOUT_SECONDS);
    }

    #[tokio::test]
    async fn echo_driver_returns_kind_in_result() {
        let driver = EchoDriver;
        let job = SimulationJob {
            job_id: "x".into(),
            kind: Some("economic".into()),
            payload: Value::Null,
        };
        let result = driver.compute(&job).await.unwrap();
        assert_eq!(result["echoed"], "economic");
    }
}
