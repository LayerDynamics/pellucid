//! Deep-forecast worker — Rust port of
//! `worldmonitor/scripts/process-deep-forecast-tasks.mjs` + the
//! `processNextDeepForecastTask` body in `seed-forecasts.mjs`.
//!
//! The actual forecast computation lives in `worldmonitor`'s 17 KLOC
//! `seed-forecasts.mjs` file (LLM prompt construction, signal aggregation,
//! market-context fetch, etc.). Replicating that monolith inside this
//! crate would couple every worker to every signal source. Instead, the
//! worker owns the **queue plumbing** (BLMOVE → claim → SETEX result) and
//! delegates the compute to a [`DeepForecastDriver`] trait. Callers wire a
//! driver inside `pellucid-relay-bin` (or a future
//! `pellucid-forecast-bin`) where the LLM + signal stacks already live.
//!
//! That keeps `pellucid-workers` as pure infrastructure — no
//! `pellucid-ml` dep, no signal logic — while preserving exact queue
//! behaviour parity with the JS source.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::WorkerError;
use crate::redis::{ListEnd, RedisClient};

/// FIFO pending queue (matches JS source).
pub const QUEUE_KEY: &str = "forecast-deep-queue:pending";
/// In-flight processing list.
pub const PROCESSING_KEY: &str = "forecast-deep-queue:processing";
/// 24 hours.
pub const RESULT_TTL_SECONDS: u64 = 86_400;
/// `processNextDeepForecastTask` is called every 30 s when idle (see
/// `seed-forecasts.mjs:36`, `FORECAST_DEEP_POLL_INTERVAL_MS`).
pub const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// `BLMOVE` block timeout (Upstash REST is non-blocking; see
/// `redis::RedisClient::blmove`).
pub const BLMOVE_TIMEOUT_SECONDS: u64 = 30;

/// One deep-forecast job. The schema is intentionally generic — the
/// driver decodes whatever fields it needs from `payload`. Required ids
/// are surfaced so the worker can dedupe + log.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeepForecastJob {
    /// Unique job identifier (caller-supplied, used as result-key suffix).
    pub job_id: String,
    /// Domain of the forecast (e.g. `"climate"`, `"conflict"`). Echoed
    /// for log filtering.
    #[serde(default)]
    pub domain: Option<String>,
    /// Caller-supplied payload — opaque to this crate.
    #[serde(default)]
    pub payload: Value,
}

/// Driver-supplied compute. Implementations live downstream (in
/// `pellucid-relay-bin` or a dedicated forecast binary).
#[async_trait]
pub trait DeepForecastDriver: Send + Sync {
    /// Run the forecast computation. Return the JSON value to write under
    /// `result.result` in the result row.
    ///
    /// # Errors
    /// Implementations should map any internal error to a
    /// [`WorkerError::Driver`] with a short stable error code; the worker
    /// writes that code into the `failed` result row.
    async fn compute(&self, job: &DeepForecastJob) -> Result<Value, WorkerError>;
}

/// Worker-loop options. Same shape as [`crate::scenario::WorkerOptions`].
#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// Run a single iteration and return.
    pub once: bool,
    /// `BLMOVE` block duration (in practice, ignored by Upstash REST).
    pub blmove_timeout_secs: u64,
    /// Sleep between empty polls — defaults to 30 s per the JS source.
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
    /// Queue was empty.
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

/// Run the deep-forecast worker. Mirrors `runDeepForecastWorker` in the
/// JS source (loop, idle-sleep, optional `--once`).
///
/// # Errors
/// Catastrophic transport failures only — per-job errors are written
/// to the result row and the loop continues.
pub async fn run_worker<D: DeepForecastDriver>(
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
                    target: "pellucid::workers::deep_forecast",
                    error = %err,
                    "deep_forecast poll error"
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
            target: "pellucid::workers::deep_forecast",
            count,
            "requeued orphaned deep_forecast jobs"
        );
    }
    Ok(())
}

/// One iteration. Public for tests.
///
/// # Errors
/// Transport failures only.
pub async fn poll_once<D: DeepForecastDriver>(
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

    let job: DeepForecastJob = match serde_json::from_str(&raw) {
        Ok(j) => j,
        Err(_) => {
            tracing::warn!(
                target: "pellucid::workers::deep_forecast",
                payload = %raw.chars().take(120).collect::<String>(),
                "unparseable deep_forecast job, discarding"
            );
            redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
            return Ok(IterationOutcome::Discarded);
        }
    };

    if job.job_id.is_empty() {
        tracing::warn!(
            target: "pellucid::workers::deep_forecast",
            "deep_forecast job has empty job_id, discarding"
        );
        redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
        return Ok(IterationOutcome::Discarded);
    }

    let result_key = format!("forecast-deep-result:{}", job.job_id);
    if let Ok(Some(_)) = redis.get_json(&result_key).await {
        tracing::info!(
            target: "pellucid::workers::deep_forecast",
            job_id = %job.job_id,
            "deep_forecast already processed, skipping"
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
                target: "pellucid::workers::deep_forecast",
                job_id = %job.job_id,
                "deep_forecast complete"
            );
            Ok(IterationOutcome::Done { job_id: job.job_id })
        }
        Err(err) => {
            tracing::error!(
                target: "pellucid::workers::deep_forecast",
                job_id = %job.job_id,
                error = %err,
                "deep_forecast driver error"
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

    struct OkDriver;
    #[async_trait]
    impl DeepForecastDriver for OkDriver {
        async fn compute(&self, _job: &DeepForecastJob) -> Result<Value, WorkerError> {
            Ok(json!({"forecast": "ok"}))
        }
    }

    #[test]
    fn options_default() {
        let opts = WorkerOptions::default();
        assert!(!opts.once);
        assert_eq!(opts.empty_backoff, POLL_INTERVAL);
        assert_eq!(opts.result_ttl_secs, RESULT_TTL_SECONDS);
    }

    #[tokio::test]
    async fn driver_returns_ok_payload() {
        let driver = OkDriver;
        let job = DeepForecastJob {
            job_id: "x".into(),
            domain: None,
            payload: Value::Null,
        };
        let result = driver.compute(&job).await.unwrap();
        assert_eq!(result["forecast"], "ok");
    }
}
