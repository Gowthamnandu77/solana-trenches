//! Per-run metrics; counters are approximate concurrent snapshots, never secrets.
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering::Relaxed},
    Arc,
};
use tokio::time::Instant;

#[derive(Default)]
pub struct Metrics {
    pub notifications_received: AtomicU64,
    pub notifications_failed: AtomicU64,
    pub notifications_dropped: AtomicU64,
    pub queue_high_water: AtomicU64,
    pub output_high_water: AtomicU64,
    pub submitted: AtomicU64,
    pub duplicates: AtomicU64,
    pub successes: AtomicU64,
    pub retries: AtomicU64,
    pub failures: AtomicU64,
    pub rate_limits: AtomicU64,
    pub decoded: AtomicU64,
    pub mentioned_only: AtomicU64,
    pub unknown: AtomicU64,
    pub processed: AtomicU64,
    pub attempts: AtomicU64,
    pub attempt_micros: AtomicU64,
    pub finished_attempts: AtomicU64,
    pub current: AtomicU64,
    pub peak: AtomicU64,
    pub jobs: AtomicU64,
    pub cancelled: AtomicU64,
}
impl Metrics {
    pub fn snapshot(&self) -> Value {
        let get = |v: &AtomicU64| v.load(Relaxed);
        let n = get(&self.finished_attempts);
        json!({
            "notifications_received":get(&self.notifications_received),
            "failed_notifications_filtered":get(&self.notifications_failed),
            "notifications_dropped":get(&self.notifications_dropped),
            "input_queue_high_water":get(&self.queue_high_water),
            "output_queue_high_water":get(&self.output_high_water),
            "transactions_submitted":get(&self.submitted), "duplicates_filtered":get(&self.duplicates),
            "successful_fetches":get(&self.successes), "retries":get(&self.retries),
            "fetch_failures":get(&self.failures), "rate_limits":get(&self.rate_limits),
            "decoded_transactions":get(&self.decoded), "mentioned_only_transactions":get(&self.mentioned_only),
            "unknown_instructions":get(&self.unknown), "processed_transactions":get(&self.processed),
            "rpc_attempts":get(&self.attempts), "current_fetch_concurrency":get(&self.current),
            "peak_fetch_concurrency":get(&self.peak), "active_jobs":get(&self.jobs),
            "cancelled_jobs":get(&self.cancelled),
            "mean_fetch_attempt_ms": if n == 0 { 0.0 } else { get(&self.attempt_micros) as f64 / n as f64 / 1000.0 }
        })
    }
}
/// RAII decrements even when the parent future is cancelled during network I/O.
pub struct AttemptGuard {
    metrics: Arc<Metrics>,
    start: Instant,
}
impl AttemptGuard {
    pub fn new(metrics: Arc<Metrics>) -> Self {
        metrics.attempts.fetch_add(1, Relaxed);
        let active = metrics.current.fetch_add(1, Relaxed) + 1;
        metrics.peak.fetch_max(active, Relaxed);
        Self {
            metrics,
            start: Instant::now(),
        }
    }
}
impl Drop for AttemptGuard {
    fn drop(&mut self) {
        self.metrics.current.fetch_sub(1, Relaxed);
        self.metrics
            .attempt_micros
            .fetch_add(self.start.elapsed().as_micros() as u64, Relaxed);
        self.metrics.finished_attempts.fetch_add(1, Relaxed);
    }
}
pub struct JobGuard {
    metrics: Arc<Metrics>,
    pub finished: bool,
}
impl JobGuard {
    pub fn new(metrics: Arc<Metrics>) -> Self {
        metrics.jobs.fetch_add(1, Relaxed);
        Self {
            metrics,
            finished: false,
        }
    }
}
impl Drop for JobGuard {
    fn drop(&mut self) {
        self.metrics.jobs.fetch_sub(1, Relaxed);
        if !self.finished {
            self.metrics.cancelled.fetch_add(1, Relaxed);
        }
    }
}
