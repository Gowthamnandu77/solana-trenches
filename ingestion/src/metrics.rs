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
    pub stale_before_fetch: AtomicU64,
    pub stale_before_momentum: AtomicU64,
    pub max_fetch_age_ms: AtomicU64,
    pub queue_wait: Histogram,
    pub queued: std::sync::Mutex<std::collections::BTreeMap<String, Instant>>,
    pub early_dedup: std::sync::Mutex<crate::dedup::EarlyDedup>,
    pub fetch_start_lag: Histogram,
    pub processing_lag: Histogram,
    pub decode_latency: Histogram,
    pub momentum_latency: Histogram,
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
        let queued = self.queued.lock().unwrap();
        let oldest = queued
            .values()
            .map(|t| t.elapsed().as_millis() as u64)
            .max()
            .unwrap_or(0);
        json!({
            "notifications_received":get(&self.notifications_received),
            "stale_before_fetch": get(&self.stale_before_fetch),
            "stale_before_momentum": get(&self.stale_before_momentum),
            "queue_occupancy": queued.len(),
            "oldest_queued_age_ms": oldest,
            "queue_wait_ms": self.queue_wait.snapshot(),
            "fetch_start_lag_ms": self.fetch_start_lag.snapshot(),
            "processing_lag_ms": self.processing_lag.snapshot(),
            "decode_latency_ms": self.decode_latency.snapshot(),
            "momentum_latency_ms": self.momentum_latency.snapshot(),
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

/// Fixed logarithmic buckets; percentile values are upper bounds in milliseconds.
#[derive(Default)]
pub struct Histogram {
    buckets: [AtomicU64; 32],
    max: AtomicU64,
}
impl Histogram {
    pub fn observe(&self, ms: u64) {
        let bucket = (64 - ms.leading_zeros()).min(31) as usize;
        self.buckets[bucket].fetch_add(1, Relaxed);
        self.max.fetch_max(ms, Relaxed);
    }
    pub fn snapshot(&self) -> Value {
        let counts: Vec<_> = self.buckets.iter().map(|b| b.load(Relaxed)).collect();
        let total: u64 = counts.iter().sum();
        let percentile = |percent: u64| {
            if total == 0 {
                return 0;
            }
            let target = (total * percent).div_ceil(100);
            let mut sum = 0;
            for (i, count) in counts.iter().enumerate() {
                sum += count;
                if sum >= target {
                    return ((1u64 << i) - 1).min(self.max.load(Relaxed));
                }
            }
            self.max.load(Relaxed)
        };
        json!({"samples":total,"p50":percentile(50),"p95":percentile(95),"max":self.max.load(Relaxed)})
    }
}
#[cfg(test)]
mod histogram_tests {
    use super::*;
    #[test]
    fn bounded_histogram() {
        let h = Histogram::default();
        for _ in 0..100_000 {
            h.observe(40);
        }
        h.observe(1000);
        let s = h.snapshot();
        assert_eq!(s["samples"], 100001);
        assert_eq!(s["p95"], 63);
        assert_eq!(s["max"], 1000);
        assert_eq!(h.buckets.len(), 32);
    }
}
