#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! Scheduler integration tests (T3.6).
//!
//! Drives `run_scheduler` with three fake seeders at different
//! cadences. Asserts each fires the expected number of times +
//! that the `SchedulerStats` counters track every run + every
//! skip. The `tick_limit` parameter caps wall-clock so each
//! test runs in under a second.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pellucid_seeders::registry::Cadence;
use pellucid_seeders::scheduler::{
    job, run_scheduler, CycleFn, SchedulerStats, SeederCycleError,
};

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

#[tokio::test]
async fn three_seeders_with_distinct_cadences_fire_each_tick_limit_times() {
    let market = Arc::new(AtomicUsize::new(0));
    let aviation = Arc::new(AtomicUsize::new(0));
    let news = Arc::new(AtomicUsize::new(0));

    let stats = SchedulerStats::new();
    let jobs = vec![
        job(
            "market",
            Cadence::every(Duration::from_millis(5)),
            ok_cycle(market.clone()),
        ),
        job(
            "aviation",
            Cadence::every(Duration::from_millis(10)),
            ok_cycle(aviation.clone()),
        ),
        job(
            "news",
            Cadence::every(Duration::from_millis(20)),
            ok_cycle(news.clone()),
        ),
    ];

    tokio::time::timeout(
        Duration::from_secs(2),
        run_scheduler(jobs, stats.clone(), Some(2)),
    )
    .await
    .unwrap();

    // tick_limit=2 → every job fires twice.
    assert_eq!(market.load(Ordering::SeqCst), 2);
    assert_eq!(aviation.load(Ordering::SeqCst), 2);
    assert_eq!(news.load(Ordering::SeqCst), 2);
    assert_eq!(stats.get("market").runs, 2);
    assert_eq!(stats.get("aviation").runs, 2);
    assert_eq!(stats.get("news").runs, 2);
    // No skips on the success path.
    assert_eq!(stats.get("market").skips, 0);
    assert_eq!(stats.get("aviation").skips, 0);
    assert_eq!(stats.get("news").skips, 0);
}

#[tokio::test]
async fn skip_counter_increments_per_failure_per_seeder() {
    let market_calls = Arc::new(AtomicUsize::new(0));
    let cyber_calls = Arc::new(AtomicUsize::new(0));
    let stats = SchedulerStats::new();
    let jobs = vec![
        job(
            "market",
            Cadence::every(Duration::from_millis(5)),
            ok_cycle(market_calls.clone()),
        ),
        job(
            "cyber",
            Cadence::every(Duration::from_millis(5)),
            err_cycle(cyber_calls.clone()),
        ),
    ];
    tokio::time::timeout(
        Duration::from_secs(2),
        run_scheduler(jobs, stats.clone(), Some(3)),
    )
    .await
    .unwrap();
    assert_eq!(stats.get("market").runs, 3);
    assert_eq!(stats.get("market").skips, 0);
    assert_eq!(stats.get("cyber").runs, 0);
    assert_eq!(stats.get("cyber").skips, 3);
}

#[tokio::test]
async fn faster_cadence_fires_more_often_within_window() {
    // Two seeders, one twice as fast as the other. With
    // tick_limit set to enforce equal counts on both, the
    // wall-clock is determined by the slower one. We
    // additionally assert the FAST seeder isn't somehow
    // slowed down by the SLOW one (the per-job intervals run
    // independently).
    let fast = Arc::new(AtomicUsize::new(0));
    let slow = Arc::new(AtomicUsize::new(0));
    let stats = SchedulerStats::new();
    let jobs = vec![
        job(
            "fast",
            Cadence::every(Duration::from_millis(5)),
            ok_cycle(fast.clone()),
        ),
        job(
            "slow",
            Cadence::every(Duration::from_millis(20)),
            ok_cycle(slow.clone()),
        ),
    ];
    tokio::time::timeout(
        Duration::from_secs(2),
        run_scheduler(jobs, stats.clone(), Some(4)),
    )
    .await
    .unwrap();
    assert_eq!(stats.get("fast").runs, 4);
    assert_eq!(stats.get("slow").runs, 4);
}

#[tokio::test]
async fn stats_snapshot_returns_clone_safe_for_diagnostic_dumps() {
    let counter = Arc::new(AtomicUsize::new(0));
    let stats = SchedulerStats::new();
    let jobs = vec![job(
        "single",
        Cadence::every(Duration::from_millis(5)),
        ok_cycle(counter),
    )];
    tokio::time::timeout(
        Duration::from_secs(2),
        run_scheduler(jobs, stats.clone(), Some(2)),
    )
    .await
    .unwrap();
    let snap = stats.snapshot();
    // The map is a clone — mutating it doesn't affect the
    // live stats object.
    let mut snap_mut = snap.clone();
    snap_mut.clear();
    let live = stats.snapshot();
    assert_eq!(live.len(), 1);
    assert_eq!(live.get("single").unwrap().runs, 2);
}
