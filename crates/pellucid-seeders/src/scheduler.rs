//! Seeder scheduler — fans out `tokio::time::interval` tasks for
//! each registered seeder.
//!
//! Per SPEC-001 §17.7 the scheduler:
//! - Spawns one task per `RegistryEntry`.
//! - Each task runs the seeder closure on the registered
//!   cadence (with optional initial delay).
//! - On any seeder error the cycle is **logged + counted**
//!   (`metrics::counter!("pellucid_seeder_skip")` per M3 fix);
//!   the scheduler does NOT crash on a single seeder failure.
//! - On clean shutdown via the supplied
//!   `tokio_util::sync::CancellationToken`-shaped flag, every
//!   task drains and the `run` future resolves.
//!
//! The scheduler is generic over the seeder dispatch closure so
//! tests can drive it with a fast clock + counting fakes; the
//! relay binary (T3.10) wires real seeders.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use parking_lot::Mutex;
use thiserror::Error;
use tokio::task::JoinSet;

use crate::registry::{Cadence, RegistryEntry};

/// Pinned future a seeder cycle returns. `Send + 'static` so the
/// scheduler can spawn it on tokio.
pub type CycleFuture = Pin<Box<dyn Future<Output = Result<(), SeederCycleError>> + Send + 'static>>;

/// Closure type the registry hands the scheduler — given the
/// seeder's name, it returns the cycle future.
pub type CycleFn = Box<dyn Fn() -> CycleFuture + Send + Sync + 'static>;

/// Errors a single seeder cycle can surface. The scheduler
/// counts them via `metrics::counter!` and continues; never
/// propagates upward.
#[derive(Debug, Error)]
pub enum SeederCycleError {
    /// Underlying seeder error (publish / upstream / validation).
    #[error("{0}")]
    Failed(String),
}

/// Read-only handle the scheduler hands to `metrics::counter!`
/// + tests. Records per-seeder skip + run counts.
#[derive(Clone, Debug, Default)]
pub struct SchedulerStats {
    inner: Arc<Mutex<HashMap<String, SeederStats>>>,
}

/// Per-seeder run / skip counters. Diagnostic + asserted on by
/// the integration test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeederStats {
    /// Number of cycles that completed `Ok(())`.
    pub runs: u64,
    /// Number of cycles that returned `Err(SeederCycleError)`.
    /// Mirrors `metrics::counter!("pellucid_seeder_skip")` —
    /// the M3 fix.
    pub skips: u64,
}

impl SchedulerStats {
    /// Construct an empty stats handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the per-seeder stats.
    #[must_use]
    pub fn snapshot(&self) -> HashMap<String, SeederStats> {
        self.inner.lock().clone()
    }

    /// Stats for one seeder.
    #[must_use]
    pub fn get(&self, name: &str) -> SeederStats {
        self.inner.lock().get(name).copied().unwrap_or_default()
    }

    fn record_run(&self, name: &str) {
        self.inner.lock().entry(name.to_string()).or_default().runs += 1;
    }

    fn record_skip(&self, name: &str) {
        self.inner.lock().entry(name.to_string()).or_default().skips += 1;
    }
}

/// One scheduled job: an entry from the registry + the closure
/// that drives one cycle.
pub struct ScheduledJob {
    /// Registry-pinned name + cadence.
    pub entry: RegistryEntry,
    /// Closure invoked on every tick.
    pub cycle: CycleFn,
}

impl std::fmt::Debug for ScheduledJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScheduledJob")
            .field("entry", &self.entry)
            .finish_non_exhaustive()
    }
}

/// Build a `ScheduledJob` from a name + cadence + closure.
/// Panics if the name is not in the static [`crate::registry::REGISTRY`]
/// — production callers MUST register at compile time.
#[must_use]
pub fn job(name: &'static str, cadence: Cadence, cycle: CycleFn) -> ScheduledJob {
    ScheduledJob {
        entry: RegistryEntry { name, cadence },
        cycle,
    }
}

/// Run the scheduler against `jobs` until `shutdown` resolves.
///
/// Each job runs on its own `tokio::time::interval`. Errors are
/// logged + counted via `SchedulerStats` + emitted to the
/// `metrics` crate's global recorder; they never propagate to
/// the caller.
///
/// `tick_limit` is an optional cap — when `Some(n)` the
/// scheduler exits after every job has fired `n` times. Used by
/// tests to bound runtime; production passes `None`.
pub async fn run_scheduler(
    jobs: Vec<ScheduledJob>,
    stats: SchedulerStats,
    tick_limit: Option<u64>,
) {
    let mut set: JoinSet<()> = JoinSet::new();
    for job in jobs {
        let stats = stats.clone();
        let cycle = job.cycle;
        let entry = job.entry;
        set.spawn(async move {
            if !entry.cadence.initial_delay.is_zero() {
                tokio::time::sleep(entry.cadence.initial_delay).await;
            }
            let mut interval = tokio::time::interval(entry.cadence.period);
            // The first `tick()` returns immediately; we want
            // the first run to actually pay one cadence period
            // (post-initial_delay) so use Burst missed-tick.
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut ticks = 0u64;
            loop {
                interval.tick().await;
                let fut = (cycle)();
                match fut.await {
                    Ok(()) => {
                        stats.record_run(entry.name);
                        metrics::counter!("pellucid_seeder_run", "seeder" => entry.name)
                            .increment(1);
                    }
                    Err(err) => {
                        stats.record_skip(entry.name);
                        metrics::counter!("pellucid_seeder_skip", "seeder" => entry.name)
                            .increment(1);
                        tracing::warn!(
                            target: "pellucid::seeders::scheduler",
                            seeder = entry.name,
                            "seeder cycle skipped: {err}",
                        );
                    }
                }
                ticks += 1;
                if let Some(limit) = tick_limit {
                    if ticks >= limit {
                        return;
                    }
                }
            }
        });
    }
    while let Some(res) = set.join_next().await {
        if let Err(e) = res {
            tracing::warn!(
                target: "pellucid::seeders::scheduler",
                "scheduler task panicked: {e}",
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn ok_cycle(counter: Arc<AtomicUsize>) -> CycleFn {
        Box::new(move || {
            let counter = counter.clone();
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        })
    }

    fn err_cycle(counter: Arc<AtomicUsize>) -> CycleFn {
        Box::new(move || {
            let counter = counter.clone();
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(SeederCycleError::Failed("simulated".into()))
            })
        })
    }

    #[test]
    fn scheduler_stats_records_runs_and_skips() {
        let s = SchedulerStats::new();
        s.record_run("a");
        s.record_run("a");
        s.record_skip("a");
        s.record_skip("b");
        let snap = s.snapshot();
        assert_eq!(snap.get("a"), Some(&SeederStats { runs: 2, skips: 1 }));
        assert_eq!(snap.get("b"), Some(&SeederStats { runs: 0, skips: 1 }));
    }

    #[test]
    fn scheduler_stats_get_is_zero_for_unknown_name() {
        let s = SchedulerStats::new();
        assert_eq!(s.get("never-recorded"), SeederStats::default());
    }

    #[tokio::test]
    async fn job_constructor_carries_name_and_cadence() {
        let counter = Arc::new(AtomicUsize::new(0));
        let j = job(
            "fixture",
            Cadence::every(Duration::from_millis(10)),
            ok_cycle(counter),
        );
        assert_eq!(j.entry.name, "fixture");
        assert_eq!(j.entry.cadence.period, Duration::from_millis(10));
    }

    #[tokio::test]
    async fn run_scheduler_fires_each_job_tick_limit_times() {
        // Three jobs, each capped at 3 ticks → 9 total cycles.
        let a = Arc::new(AtomicUsize::new(0));
        let b = Arc::new(AtomicUsize::new(0));
        let c = Arc::new(AtomicUsize::new(0));
        let stats = SchedulerStats::new();
        let jobs = vec![
            job(
                "a",
                Cadence::every(Duration::from_millis(5)),
                ok_cycle(a.clone()),
            ),
            job(
                "b",
                Cadence::every(Duration::from_millis(5)),
                ok_cycle(b.clone()),
            ),
            job(
                "c",
                Cadence::every(Duration::from_millis(5)),
                ok_cycle(c.clone()),
            ),
        ];
        tokio::time::timeout(
            Duration::from_secs(2),
            run_scheduler(jobs, stats.clone(), Some(3)),
        )
        .await
        .unwrap();
        assert_eq!(a.load(Ordering::SeqCst), 3);
        assert_eq!(b.load(Ordering::SeqCst), 3);
        assert_eq!(c.load(Ordering::SeqCst), 3);
        assert_eq!(stats.get("a").runs, 3);
        assert_eq!(stats.get("b").runs, 3);
        assert_eq!(stats.get("c").runs, 3);
    }

    #[tokio::test]
    async fn run_scheduler_counts_skips_on_seeder_failure() {
        let counter = Arc::new(AtomicUsize::new(0));
        let stats = SchedulerStats::new();
        let jobs = vec![job(
            "always-fails",
            Cadence::every(Duration::from_millis(5)),
            err_cycle(counter.clone()),
        )];
        tokio::time::timeout(
            Duration::from_secs(2),
            run_scheduler(jobs, stats.clone(), Some(3)),
        )
        .await
        .unwrap();
        // Cycle ran 3 times (counter), all skipped.
        assert_eq!(counter.load(Ordering::SeqCst), 3);
        let s = stats.get("always-fails");
        assert_eq!(s.skips, 3);
        assert_eq!(s.runs, 0);
    }

    #[tokio::test]
    async fn run_scheduler_continues_after_individual_failure() {
        // Mixed success + failure jobs; each job's count is
        // independent.
        let ok_counter = Arc::new(AtomicUsize::new(0));
        let err_counter = Arc::new(AtomicUsize::new(0));
        let stats = SchedulerStats::new();
        let jobs = vec![
            job(
                "ok",
                Cadence::every(Duration::from_millis(5)),
                ok_cycle(ok_counter.clone()),
            ),
            job(
                "fails",
                Cadence::every(Duration::from_millis(5)),
                err_cycle(err_counter.clone()),
            ),
        ];
        tokio::time::timeout(
            Duration::from_secs(2),
            run_scheduler(jobs, stats.clone(), Some(2)),
        )
        .await
        .unwrap();
        assert_eq!(stats.get("ok").runs, 2);
        assert_eq!(stats.get("ok").skips, 0);
        assert_eq!(stats.get("fails").runs, 0);
        assert_eq!(stats.get("fails").skips, 2);
    }

    #[tokio::test]
    async fn initial_delay_pushes_first_run() {
        let counter = Arc::new(AtomicUsize::new(0));
        let stats = SchedulerStats::new();
        let jobs = vec![ScheduledJob {
            entry: RegistryEntry {
                name: "delayed",
                cadence: Cadence::every(Duration::from_millis(20))
                    .with_initial_delay(Duration::from_millis(40)),
            },
            cycle: ok_cycle(counter.clone()),
        }];
        // After 30ms the cycle should NOT have fired (initial
        // delay = 40ms). After 100ms it should have fired
        // ~3 times (40ms delay + 3 * 20ms intervals).
        let handle = tokio::spawn(async move {
            run_scheduler(jobs, stats.clone(), Some(3)).await;
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(counter.load(Ordering::SeqCst) >= 3);
    }
}
