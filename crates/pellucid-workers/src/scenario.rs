//! Scenario worker — port of `worldmonitor/scripts/scenario-worker.mjs`.
//!
//! Atomically dequeues scenario jobs from the Upstash queue, runs
//! [`compute_scenario`], and writes results back with a 24-hour TTL.
//!
//! # Algorithm (parity with the JS source, lines 188-315)
//! 1. Resolve the scenario template by id.
//! 2. Read live chokepoint statuses from the supply-chain cache.
//! 3. Build the (reporter × HS2) cache-key matrix once and pipeline-GET.
//! 4. For each row:
//!    - **Tariff shock** (`affectedChokepointIds.is_empty()`): use
//!      `vulnerabilityIndex` as the proxy and apply
//!      `costShockMultiplier`.
//!    - **Physical disruption**: for every exposure entry whose
//!      `chokepointId` is in `affectedChokepointIds`, compute
//!      `exposureScore × disruptionPct% × costShockMultiplier`.
//! 5. Aggregate by reporter, sort desc, take top 20, derive `impactPct`
//!    relative to the worst-hit country (capped at 100).
//!
//! # Templates
//! [`SCENARIO_TEMPLATES`] is an inline copy of the six templates from
//! `worldmonitor/server/worldmonitor/supply-chain/v1/scenario-templates.ts`,
//! mirrored exactly per the JS worker's comment "Keep in sync with…".

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::WorkerError;
use crate::redis::RedisClient;

/// FIFO pending queue.
pub const QUEUE_KEY: &str = "scenario-queue:pending";
/// In-flight processing list (for orphan recovery).
pub const PROCESSING_KEY: &str = "scenario-queue:processing";
/// 24 hours.
pub const RESULT_TTL_SECONDS: u64 = 86_400;
/// Block duration for `BLMOVE` (Upstash REST returns null immediately for
/// empty queues — see `redis::RedisClient::blmove` doc).
pub const BLMOVE_TIMEOUT_SECONDS: u64 = 30;
/// Sleep between empty polls so the worker doesn't busy-loop.
pub const EMPTY_POLL_BACKOFF: Duration = Duration::from_secs(5);
/// Sleep between BLMOVE error retries.
pub const ERROR_BACKOFF: Duration = Duration::from_secs(5);

/// Scenario template definition. Subset of the TS shape — only fields the
/// worker needs.
#[derive(Debug, Clone)]
pub struct ScenarioTemplate {
    /// Stable scenario id (e.g. `"taiwan-strait-full-closure"`).
    pub id: &'static str,
    /// Chokepoint ids whose disruption this scenario simulates. Empty for
    /// tariff-shock scenarios that don't close any physical lane.
    pub affected_chokepoint_ids: &'static [&'static str],
    /// 0..=100, percent of capacity removed.
    pub disruption_pct: u32,
    /// Duration in days — surfaced in the response, not used in scoring.
    pub duration_days: u32,
    /// HS2 chapters this scenario is scoped to. `None` = all chapters
    /// (`"01"..="99"`).
    pub affected_hs2: Option<&'static [&'static str]>,
    /// Multiplier applied to every score (e.g. 1.45 = +45 % cost shock).
    pub cost_shock_multiplier: f64,
}

/// Inline copy — keep in sync with the TS source.
pub static SCENARIO_TEMPLATES: &[ScenarioTemplate] = &[
    ScenarioTemplate {
        id: "taiwan-strait-full-closure",
        affected_chokepoint_ids: &["taiwan_strait"],
        disruption_pct: 100,
        duration_days: 30,
        affected_hs2: Some(&["84", "85", "87"]),
        cost_shock_multiplier: 1.45,
    },
    ScenarioTemplate {
        id: "suez-bab-simultaneous",
        affected_chokepoint_ids: &["suez", "bab_el_mandeb"],
        disruption_pct: 80,
        duration_days: 60,
        affected_hs2: None,
        cost_shock_multiplier: 1.35,
    },
    ScenarioTemplate {
        id: "panama-drought-50pct",
        affected_chokepoint_ids: &["panama"],
        disruption_pct: 50,
        duration_days: 90,
        affected_hs2: None,
        cost_shock_multiplier: 1.22,
    },
    ScenarioTemplate {
        id: "hormuz-tanker-blockade",
        affected_chokepoint_ids: &["hormuz_strait"],
        disruption_pct: 100,
        duration_days: 14,
        affected_hs2: Some(&["27", "29"]),
        cost_shock_multiplier: 2.10,
    },
    ScenarioTemplate {
        id: "russia-baltic-grain-suspension",
        affected_chokepoint_ids: &["bosphorus", "dover_strait"],
        disruption_pct: 100,
        duration_days: 180,
        affected_hs2: Some(&["10", "12"]),
        cost_shock_multiplier: 1.55,
    },
    ScenarioTemplate {
        id: "us-tariff-escalation-electronics",
        affected_chokepoint_ids: &[],
        disruption_pct: 0,
        duration_days: 365,
        affected_hs2: Some(&["85"]),
        cost_shock_multiplier: 1.50,
    },
];

/// The six v1-seeded reporters (per scenario-worker.mjs:222).
pub const SEEDED_REPORTERS: &[&str] = &["US", "CN", "RU", "IR", "IN", "TW"];

/// One scenario job as it sits on the pending queue.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScenarioJob {
    /// `scenario:<13-digit-ms>:<8-char-alphanum>`.
    pub job_id: String,
    /// Template id.
    pub scenario_id: String,
    /// `null` = all reporters; otherwise a 2-letter ISO code.
    pub iso2: Option<String>,
    /// Job enqueue timestamp (ms epoch).
    pub enqueued_at: i64,
}

/// One row of the per-country impact ranking returned in the result.
#[derive(Debug, Clone, Serialize)]
pub struct CountryImpact {
    /// Reporter ISO 3166-1 alpha-2.
    pub iso2: String,
    /// Sum of `adjustedImpact` across HS2 chapters and chokepoints.
    pub total_impact: f64,
    /// Relative share of the worst-hit country (0..=100).
    pub impact_pct: u32,
}

/// Output shape written to `scenario-result:<job_id>` on success.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    /// Echo of the requested scenario id.
    pub scenario_id: String,
    /// Subset of the template (the JS worker calls this `template`).
    pub template: TemplateSummary,
    /// Echoed for the UI's chokepoint badges.
    pub affected_chokepoint_ids: Vec<String>,
    /// Live disruption score for each affected chokepoint at compute time.
    pub current_disruption_scores: HashMap<String, Option<f64>>,
    /// Top-20 reporters by total impact, ranked desc.
    pub top_impact_countries: Vec<CountryImpact>,
    /// HS2 chapters in scope (mirrors the template).
    pub affected_hs2: Option<Vec<String>>,
    /// `Some(iso2)` if the request was scoped to a single reporter.
    pub scoped_iso2: Option<String>,
    /// Compute timestamp (ms epoch).
    pub computed_at: i64,
}

/// Subset of [`ScenarioTemplate`] surfaced in the JSON result.
#[derive(Debug, Clone, Serialize)]
pub struct TemplateSummary {
    /// `affected_chokepoint_ids.join("+")` or `"tariff_shock"` when empty.
    pub name: String,
    /// 0..=100.
    pub disruption_pct: u32,
    /// Days (informational).
    pub duration_days: u32,
    /// e.g. 1.45.
    pub cost_shock_multiplier: f64,
}

/// Worker-loop options — exposed so the relay binary can wire them from
/// `Config` and tests can pass shorter cadences.
#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// Run a single iteration and return. The worker exits as soon as one
    /// job has been processed (or the queue was empty).
    pub once: bool,
    /// How long the worker waits on `BLMOVE` (Upstash REST is non-blocking
    /// in practice — see [`BLMOVE_TIMEOUT_SECONDS`]).
    pub blmove_timeout_secs: u64,
    /// Sleep between empty polls.
    pub empty_backoff: Duration,
    /// Result TTL.
    pub result_ttl_secs: u64,
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            once: false,
            blmove_timeout_secs: BLMOVE_TIMEOUT_SECONDS,
            empty_backoff: EMPTY_POLL_BACKOFF,
            result_ttl_secs: RESULT_TTL_SECONDS,
        }
    }
}

/// Per-iteration outcome. Useful for tests + diagnostic logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IterationOutcome {
    /// Queue was empty.
    Idle,
    /// Job processed successfully.
    Done {
        /// Echoed job id (for log correlation).
        job_id: String,
    },
    /// Job dequeued but failed validation — discarded.
    Discarded,
    /// Job dequeued but `compute_scenario` returned an error — `failed`
    /// row written.
    Failed {
        /// Echoed job id.
        job_id: String,
    },
    /// Job dequeued but result already existed (idempotent dedupe).
    AlreadyProcessed {
        /// Echoed job id.
        job_id: String,
    },
}

/// Run the scenario worker until either `options.once == true` or the
/// process is asked to shut down. Production callers pass
/// `WorkerOptions::default()`.
///
/// # Errors
/// Only catastrophic / unrecoverable conditions surface here. Per-job
/// failures (validation, compute) are written to the result key with
/// `status: failed` and the loop continues.
pub async fn run_worker(
    redis: &RedisClient,
    options: WorkerOptions,
) -> Result<(), WorkerError> {
    requeue_orphaned_jobs(redis).await?;

    loop {
        match poll_once(redis, &options).await {
            Ok(IterationOutcome::Idle) if options.once => return Ok(()),
            Ok(IterationOutcome::Idle) => {
                tokio::time::sleep(options.empty_backoff).await;
            }
            Ok(_) if options.once => return Ok(()),
            Ok(_) => {}
            Err(err) => {
                tracing::error!(
                    target: "pellucid::workers::scenario",
                    error = %err,
                    "scenario worker poll error"
                );
                if options.once {
                    return Err(err);
                }
                tokio::time::sleep(ERROR_BACKOFF).await;
            }
        }
    }
}

/// Drain `PROCESSING_KEY` back into `QUEUE_KEY` at startup. Mirrors
/// `requeueOrphanedJobs` in the JS source.
///
/// # Errors
/// Transport failures.
pub async fn requeue_orphaned_jobs(redis: &RedisClient) -> Result<(), WorkerError> {
    let mut count = 0u32;
    loop {
        let moved = redis
            .lmove(
                PROCESSING_KEY,
                QUEUE_KEY,
                crate::redis::ListEnd::Right,
                crate::redis::ListEnd::Left,
            )
            .await?;
        if moved.is_some() {
            count += 1;
        } else {
            break;
        }
    }
    if count > 0 {
        tracing::info!(
            target: "pellucid::workers::scenario",
            count,
            "requeued orphaned scenario jobs"
        );
    }
    Ok(())
}

/// One iteration. Public so a test (or a future single-shot CLI command)
/// can drive the loop deterministically.
///
/// # Errors
/// Transport failures only — per-job validation / compute errors are
/// converted to `Discarded` / `Failed` outcomes.
pub async fn poll_once(
    redis: &RedisClient,
    options: &WorkerOptions,
) -> Result<IterationOutcome, WorkerError> {
    let raw = redis
        .blmove(QUEUE_KEY, PROCESSING_KEY, options.blmove_timeout_secs)
        .await?;
    let Some(raw) = raw else {
        return Ok(IterationOutcome::Idle);
    };

    let job: ScenarioJob = match serde_json::from_str(&raw) {
        Ok(j) => j,
        Err(_) => {
            tracing::warn!(
                target: "pellucid::workers::scenario",
                payload = %raw.chars().take(100).collect::<String>(),
                "unparseable scenario job, discarding"
            );
            redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
            return Ok(IterationOutcome::Discarded);
        }
    };

    if !validate_job(&job) {
        tracing::warn!(
            target: "pellucid::workers::scenario",
            job_id = %job.job_id,
            "scenario job failed field validation, discarding"
        );
        redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
        return Ok(IterationOutcome::Discarded);
    }

    let result_key = format!("scenario-result:{}", job.job_id);

    if let Ok(Some(_)) = redis.get_json(&result_key).await {
        tracing::info!(
            target: "pellucid::workers::scenario",
            job_id = %job.job_id,
            "scenario already processed, skipping"
        );
        redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
        return Ok(IterationOutcome::AlreadyProcessed { job_id: job.job_id });
    }

    let processing_state = json!({ "status": "processing", "startedAt": now_ms() });
    redis
        .setex(&result_key, options.result_ttl_secs, &processing_state)
        .await
        .ok();

    let compute = compute_scenario(redis, &job.scenario_id, job.iso2.as_deref()).await;

    match compute {
        Ok(result) => {
            let payload = json!({
                "status": "done",
                "result": result,
                "completedAt": now_ms(),
            });
            redis
                .setex(&result_key, options.result_ttl_secs, &payload)
                .await?;
            tracing::info!(
                target: "pellucid::workers::scenario",
                job_id = %job.job_id,
                countries = result.top_impact_countries.len(),
                "scenario job complete"
            );
            redis.lrem_first(PROCESSING_KEY, &raw).await.ok();
            Ok(IterationOutcome::Done { job_id: job.job_id })
        }
        Err(err) => {
            tracing::error!(
                target: "pellucid::workers::scenario",
                job_id = %job.job_id,
                error = %err,
                "scenario compute failed"
            );
            let payload = json!({
                "status": "failed",
                "error": "computation_error",
                "failedAt": now_ms(),
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

/// Compute the impact of a scenario across countries and HS2 sectors.
///
/// # Errors
/// - [`WorkerError::InvalidJob`] for unknown scenario ids.
/// - Transport failures.
pub async fn compute_scenario(
    redis: &RedisClient,
    scenario_id: &str,
    iso2: Option<&str>,
) -> Result<ScenarioResult, WorkerError> {
    let template = SCENARIO_TEMPLATES
        .iter()
        .find(|t| t.id == scenario_id)
        .ok_or_else(|| WorkerError::InvalidJob(format!("unknown scenario: {scenario_id}")))?;

    // Live chokepoint cache (best-effort — may be absent in dev).
    let cp_data = redis
        .get_json("supply_chain:chokepoints:v4")
        .await
        .unwrap_or(None);
    let mut current_scores: HashMap<String, Option<f64>> = HashMap::new();
    if let Some(Value::Object(map)) = cp_data {
        if let Some(Value::Array(cps)) = map.get("chokepoints") {
            for cp in cps {
                if let (Some(Value::String(id)), Some(Value::Number(n))) =
                    (cp.get("id"), cp.get("disruptionScore"))
                {
                    if let Some(f) = n.as_f64() {
                        current_scores.insert(id.clone(), Some(f));
                    }
                }
            }
        }
    }

    let reporters: Vec<String> = match iso2 {
        Some(c) => vec![c.to_string()],
        None => SEEDED_REPORTERS.iter().map(|s| (*s).to_string()).collect(),
    };

    let is_tariff_shock = template.affected_chokepoint_ids.is_empty();

    let hs2_chapters: Vec<String> = match template.affected_hs2 {
        Some(chs) => chs.iter().map(|s| (*s).to_string()).collect(),
        None => (1..=99).map(|i| format!("{i:02}")).collect(),
    };

    let mut keys = Vec::with_capacity(reporters.len() * hs2_chapters.len());
    for reporter in &reporters {
        for hs2 in &hs2_chapters {
            keys.push(format!("supply-chain:exposure:{reporter}:{hs2}:v1"));
        }
    }

    let pipeline_results = redis.pipeline_get(&keys).await?;

    #[derive(Debug, Clone)]
    struct Impact {
        iso2: String,
        adjusted_impact: f64,
    }

    let mut impacts: Vec<Impact> = Vec::new();
    let mut idx = 0usize;
    for reporter in &reporters {
        for _hs2 in &hs2_chapters {
            let data = pipeline_results.get(idx).cloned().flatten();
            idx += 1;
            let Some(Value::Object(row)) = data else {
                continue;
            };

            if is_tariff_shock {
                let vuln = row
                    .get("vulnerabilityIndex")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                if vuln > 0.0 {
                    impacts.push(Impact {
                        iso2: reporter.clone(),
                        adjusted_impact: vuln * template.cost_shock_multiplier,
                    });
                }
                continue;
            }

            let Some(Value::Array(exposures)) = row.get("exposures") else {
                continue;
            };
            for entry in exposures {
                let Some(Value::Object(e)) = Some(entry) else { continue; };
                let Some(Value::String(cp_id)) = e.get("chokepointId") else {
                    continue;
                };
                if !template.affected_chokepoint_ids.contains(&cp_id.as_str()) {
                    continue;
                }
                let exposure_score = e
                    .get("exposureScore")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                if exposure_score <= 0.0 {
                    continue;
                }
                let adjusted_impact = exposure_score
                    * (f64::from(template.disruption_pct) / 100.0)
                    * template.cost_shock_multiplier;
                impacts.push(Impact {
                    iso2: reporter.clone(),
                    adjusted_impact,
                });
            }
        }
    }

    let mut by_country: HashMap<String, f64> = HashMap::new();
    for item in impacts {
        *by_country.entry(item.iso2).or_insert(0.0) += item.adjusted_impact;
    }

    let mut sorted: Vec<(String, f64)> = by_country.into_iter().collect();
    sorted.sort_by(|a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted.truncate(20);
    let max_impact = sorted.first().map(|(_, v)| *v).unwrap_or(0.0).max(1.0);
    let top_impact_countries: Vec<CountryImpact> = sorted
        .into_iter()
        .map(|(iso2, total)| CountryImpact {
            iso2,
            total_impact: total,
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            impact_pct: ((total / max_impact) * 100.0).round().clamp(0.0, 100.0) as u32,
        })
        .collect();

    let template_summary = TemplateSummary {
        name: if template.affected_chokepoint_ids.is_empty() {
            "tariff_shock".into()
        } else {
            template.affected_chokepoint_ids.join("+")
        },
        disruption_pct: template.disruption_pct,
        duration_days: template.duration_days,
        cost_shock_multiplier: template.cost_shock_multiplier,
    };

    let current = template
        .affected_chokepoint_ids
        .iter()
        .map(|id| ((*id).to_string(), current_scores.get(*id).copied().flatten()))
        .collect();

    Ok(ScenarioResult {
        scenario_id: scenario_id.to_string(),
        template: template_summary,
        affected_chokepoint_ids: template
            .affected_chokepoint_ids
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        current_disruption_scores: current,
        top_impact_countries,
        affected_hs2: template
            .affected_hs2
            .map(|chs| chs.iter().map(|s| (*s).to_string()).collect()),
        scoped_iso2: iso2.map(str::to_string),
        computed_at: now_ms(),
    })
}

fn validate_job(job: &ScenarioJob) -> bool {
    let id_ok = job.job_id.starts_with("scenario:") && {
        let parts: Vec<&str> = job.job_id.split(':').collect();
        parts.len() == 3
            && parts[1].len() == 13
            && parts[1].chars().all(|c| c.is_ascii_digit())
            && parts[2].len() == 8
            && parts[2].chars().all(|c| c.is_ascii_alphanumeric())
    };
    let scenario_ok = !job.scenario_id.is_empty();
    let iso_ok = match &job.iso2 {
        None => true,
        Some(c) => c.len() == 2 && c.chars().all(|ch| ch.is_ascii_uppercase()),
    };
    id_ok && scenario_ok && iso_ok
}

/// Wall-clock timestamp in ms since UNIX epoch. Shared by every worker
/// loop's result writes — exposed at crate root via `crate::now_ms`.
#[must_use]
#[allow(clippy::cast_possible_wrap)]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;

    fn job(id: &str, scen: &str, iso: Option<&str>) -> ScenarioJob {
        ScenarioJob {
            job_id: id.into(),
            scenario_id: scen.into(),
            iso2: iso.map(str::to_string),
            enqueued_at: 0,
        }
    }

    #[test]
    fn validate_job_accepts_well_formed_payload() {
        assert!(validate_job(&job(
            "scenario:1700000000000:abcd1234",
            "taiwan-strait-full-closure",
            None
        )));
        assert!(validate_job(&job(
            "scenario:1700000000000:abcd1234",
            "x",
            Some("US")
        )));
    }

    #[test]
    fn validate_job_rejects_bad_id() {
        assert!(!validate_job(&job("notscenario:x:y", "x", None)));
        assert!(!validate_job(&job("scenario:short:abcd1234", "x", None)));
        assert!(!validate_job(&job(
            "scenario:1700000000000:short",
            "x",
            None
        )));
        // also rejects uppercase suffix (regex is [a-z0-9]{8})
        assert!(!validate_job(&job(
            "scenario:1700000000000:abcd123!",
            "x",
            None
        )));
    }

    #[test]
    fn validate_job_rejects_bad_iso() {
        assert!(!validate_job(&job(
            "scenario:1700000000000:abcd1234",
            "x",
            Some("us")
        )));
        assert!(!validate_job(&job(
            "scenario:1700000000000:abcd1234",
            "x",
            Some("USA")
        )));
    }

    #[test]
    fn validate_job_rejects_empty_scenario_id() {
        assert!(!validate_job(&job(
            "scenario:1700000000000:abcd1234",
            "",
            None
        )));
    }

    #[test]
    fn templates_match_js_inline_copy() {
        // Spot checks — names + multipliers must be byte-equal to the JS
        // SCENARIO_TEMPLATES list. The handler depends on these strings.
        let by_id: HashMap<&str, &ScenarioTemplate> =
            SCENARIO_TEMPLATES.iter().map(|t| (t.id, t)).collect();
        assert_eq!(by_id.len(), 6);
        let taiwan = by_id["taiwan-strait-full-closure"];
        assert_eq!(taiwan.disruption_pct, 100);
        assert_eq!(taiwan.duration_days, 30);
        assert_eq!(taiwan.affected_chokepoint_ids, &["taiwan_strait"]);
        assert_eq!(taiwan.cost_shock_multiplier, 1.45);

        let tariff = by_id["us-tariff-escalation-electronics"];
        assert!(tariff.affected_chokepoint_ids.is_empty());
        assert_eq!(tariff.affected_hs2, Some(&["85"][..]));
        assert_eq!(tariff.cost_shock_multiplier, 1.50);
    }

    #[test]
    fn worker_options_default_values() {
        let opts = WorkerOptions::default();
        assert!(!opts.once);
        assert_eq!(opts.blmove_timeout_secs, BLMOVE_TIMEOUT_SECONDS);
        assert_eq!(opts.result_ttl_secs, RESULT_TTL_SECONDS);
    }
}
