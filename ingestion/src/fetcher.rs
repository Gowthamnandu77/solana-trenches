use crate::{
    config::Config,
    dedup::remember_signature,
    listener::LogEvent,
    metrics::{AttemptGuard, JobGuard, Metrics},
    rpc::{FetchError, HttpTransport, Transport},
};
use futures_util::{stream::FuturesUnordered, StreamExt};
use solana_sdk::signature::Signature;
use std::{
    collections::{HashSet, VecDeque},
    str::FromStr,
    sync::{
        atomic::{AtomicU64, Ordering::Relaxed},
        Arc,
    },
};
use tokio::{
    sync::{mpsc, Mutex},
    time::{sleep, sleep_until, timeout, Duration, Instant},
};

pub struct FetchedEvent {
    pub log: LogEvent,
    pub transaction: serde_json::Value,
    pub block_time: Option<i64>,
    pub fetch_started_unix_ms: i64,
    pub fetch_completed_unix_ms: i64,
    pub slot: u64,
}
pub static FETCH_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Shared spacing and cooldown across *all* jobs, including retries. No mutex
/// is held during network I/O or sleeps. Waiters recheck after a 429 extends it.
struct RequestGate {
    next: Mutex<Instant>,
    spacing: Duration,
}
impl RequestGate {
    fn new(spacing: Duration) -> Self {
        Self {
            next: Mutex::new(Instant::now()),
            spacing,
        }
    }
    async fn acquire(&self) {
        loop {
            let mut next = self.next.lock().await;
            if Instant::now() >= *next {
                *next = Instant::now() + self.spacing;
                return;
            }
            let wake = *next;
            drop(next);
            sleep_until(wake).await;
        }
    }
    async fn cooldown(&self, duration: Duration) {
        let mut next = self.next.lock().await;
        *next = (*next).max(Instant::now() + duration);
    }
}

pub async fn run(
    config: Config,
    input: mpsc::Receiver<LogEvent>,
    output: mpsc::Sender<FetchedEvent>,
    metrics: Arc<Metrics>,
) {
    metrics
        .max_fetch_age_ms
        .store(config.max_fetch_start_age_ms, Relaxed);
    let Ok(rpc) = HttpTransport::new(config.http_url) else {
        eprintln!("Unable to initialize HTTP client (details redacted)");
        return;
    };
    dispatch(
        Arc::new(rpc),
        input,
        output,
        config.max_fetch_concurrency,
        Duration::from_millis(config.rpc_request_interval_ms),
        metrics,
    )
    .await;
}

/// FuturesUnordered contains at most `limit` jobs: no per-notification spawn,
/// no unbounded semaphore waiters, and output backpressure retains a job slot.
/// Dropping this future cancels every child future synchronously.
async fn dispatch<T: Transport>(
    rpc: Arc<T>,
    mut input: mpsc::Receiver<LogEvent>,
    output: mpsc::Sender<FetchedEvent>,
    limit: usize,
    spacing: Duration,
    metrics: Arc<Metrics>,
) {
    assert!((1..=64).contains(&limit));
    let gate = Arc::new(RequestGate::new(spacing));
    let (mut seen, mut queue, mut pending) = (HashSet::new(), VecDeque::new(), HashSet::new());
    let mut jobs = FuturesUnordered::new();
    let mut input_closed = false;
    loop {
        if input_closed && jobs.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            _ = output.closed() => break,
            Some(signature) = jobs.next(), if !jobs.is_empty() => { pending.remove(&signature); }
            event = input.recv(), if !input_closed && jobs.len() < limit => {
                let Some(log) = event else { input_closed = true; continue; };
                metrics.queued.lock().unwrap().remove(&log.signature);
                metrics.queue_wait.observe(log.received_at.elapsed().as_millis() as u64);
                let max_age = metrics.max_fetch_age_ms.load(Relaxed);
                if max_age > 0 && log.received_at.elapsed().as_millis() > max_age as u128 {
                    metrics.stale_before_fetch.fetch_add(1, Relaxed);
                    continue;
                }
                if pending.contains(&log.signature) || !remember_signature(&log.signature, &mut seen, &mut queue) {
                    metrics.duplicates.fetch_add(1, Relaxed); continue;
                }
                if Signature::from_str(&log.signature).is_err() { continue; }
                pending.insert(log.signature.clone());
                metrics.submitted.fetch_add(1, Relaxed);
                let guard = JobGuard::new(metrics.clone());
                jobs.push(fetch_one(rpc.clone(), log, output.clone(), gate.clone(), metrics.clone(), guard));
            }
        }
    }
}
async fn fetch_one<T: Transport>(
    rpc: Arc<T>,
    log: LogEvent,
    output: mpsc::Sender<FetchedEvent>,
    gate: Arc<RequestGate>,
    metrics: Arc<Metrics>,
    mut job: JobGuard,
) -> String {
    let signature = log.signature.clone();
    for attempt in 0..5 {
        gate.acquire().await;
        let age = log.received_at.elapsed().as_millis() as u64;
        let max_age = metrics.max_fetch_age_ms.load(Relaxed);
        if max_age > 0 && age > max_age {
            metrics.stale_before_fetch.fetch_add(1, Relaxed);
            job.finished = true;
            return signature;
        }
        metrics.fetch_start_lag.observe(age);
        if attempt > 0 {
            metrics.retries.fetch_add(1, Relaxed);
        }
        let fetch_started_unix_ms = chrono::Utc::now().timestamp_millis();
        let active = AttemptGuard::new(metrics.clone());
        let result = timeout(Duration::from_secs(12), rpc.fetch(&signature))
            .await
            .unwrap_or(Err(FetchError::Transient));
        drop(active);
        match result {
            Ok(tx) => {
                metrics.successes.fetch_add(1, Relaxed);
                if output
                    .send(FetchedEvent {
                        log,
                        transaction: tx.json,
                        fetch_started_unix_ms,
                        fetch_completed_unix_ms: chrono::Utc::now().timestamp_millis(),
                        block_time: tx.block_time,
                        slot: tx.slot,
                    })
                    .await
                    .is_ok()
                {
                    metrics
                        .output_high_water
                        .fetch_max((output.max_capacity() - output.capacity()) as u64, Relaxed);
                    job.finished = true;
                }
                return signature;
            }
            Err(error) => {
                let delay = Duration::from_millis(500 * (1 << attempt));
                if let FetchError::RateLimited(retry_after) = error {
                    metrics.rate_limits.fetch_add(1, Relaxed);
                    // Even the last failed attempt cools down other jobs.
                    gate.cooldown(retry_after.max(delay)).await;
                }
                if matches!(error, FetchError::Permanent) || attempt == 4 {
                    break;
                }
                sleep(delay).await;
            }
        }
    }
    metrics.failures.fetch_add(1, Relaxed);
    FETCH_FAILURES.fetch_add(1, Relaxed);
    job.finished = true;
    signature
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::Transaction;
    use std::{collections::HashMap, sync::Mutex as StdMutex};
    struct Mock {
        calls: StdMutex<HashMap<String, usize>>,
        failures: usize,
        error: FetchError,
        delay: Duration,
    }
    impl Mock {
        fn new(failures: usize, error: FetchError, delay: Duration) -> Self {
            Self {
                calls: StdMutex::new(HashMap::new()),
                failures,
                error,
                delay,
            }
        }
    }
    impl Transport for Mock {
        async fn fetch(&self, signature: &str) -> Result<Transaction, FetchError> {
            let call = {
                let mut calls = self.calls.lock().unwrap();
                let count = calls.entry(signature.to_owned()).or_default();
                *count += 1;
                *count
            };
            sleep(self.delay).await;
            if call <= self.failures {
                return Err(self.error);
            }
            Ok(Transaction {
                json: serde_json::json!({"meta":{"err":null}}),
                slot: 1,
                block_time: None,
            })
        }
    }
    fn event(id: u8) -> LogEvent {
        LogEvent {
            source: "raydium_cpmm",
            program_id: "cp",
            signature: bs58::encode([id; 64]).into_string(),
            slot: 1,
            logs: vec![],
            received_at: Instant::now(),
            received_unix_ms: 0,
        }
    }
    #[tokio::test(start_paused = true)]
    async fn stale_queue_and_gate_wait_skip_rpc() {
        let rpc = Arc::new(Mock::new(0, FetchError::Transient, Duration::ZERO));
        let metrics = Arc::new(Metrics::default());
        metrics.max_fetch_age_ms.store(100, Relaxed);
        let (tx, rx) = mpsc::channel(2);
        let (output, mut received) = mpsc::channel(2);
        tx.send(event(1)).await.unwrap();
        tokio::time::advance(Duration::from_millis(101)).await;
        drop(tx);
        dispatch(rpc.clone(), rx, output, 1, Duration::ZERO, metrics.clone()).await;
        assert!(received.recv().await.is_none());
        assert!(rpc.calls.lock().unwrap().is_empty());
        assert_eq!(metrics.stale_before_fetch.load(Relaxed), 1);
        assert_eq!(metrics.queue_wait.snapshot()["max"], 101);
    }
    #[tokio::test(start_paused = true)]
    async fn bounded_concurrency_duplicates_and_retry_deliver_each_transaction_once() {
        let rpc = Arc::new(Mock::new(1, FetchError::Transient, Duration::from_secs(1)));
        let metrics = Arc::new(Metrics::default());
        let (tx, rx) = mpsc::channel(8);
        let (output, mut received) = mpsc::channel(1);
        for id in [1, 2, 1, 3, 4, 2] {
            tx.send(event(id)).await.unwrap();
        }
        drop(tx);
        let worker = tokio::spawn(dispatch(
            rpc.clone(),
            rx,
            output,
            3,
            Duration::ZERO,
            metrics.clone(),
        ));
        let mut signatures = HashSet::new();
        while let Some(result) = received.recv().await {
            assert!(
                signatures.insert(result.log.signature),
                "duplicate delivered to momentum coordinator"
            );
        }
        worker.await.unwrap();
        assert_eq!(signatures.len(), 4);
        assert_eq!(metrics.submitted.load(Relaxed), 4);
        assert_eq!(metrics.duplicates.load(Relaxed), 2);
        assert_eq!(metrics.retries.load(Relaxed), 4);
        assert_eq!(metrics.successes.load(Relaxed), 4);
        assert_eq!(metrics.peak.load(Relaxed), 3);
        assert_eq!(metrics.current.load(Relaxed), 0);
        assert_eq!(metrics.jobs.load(Relaxed), 0);
        assert!(rpc.calls.lock().unwrap().values().all(|n| *n == 2));
    }
    #[tokio::test(start_paused = true)]
    async fn full_input_capacity_and_shutdown_cancel_pending_jobs() {
        let metrics = Arc::new(Metrics::default());
        let rpc = Arc::new(Mock::new(
            0,
            FetchError::Transient,
            Duration::from_secs(3600),
        ));
        let (tx, rx) = mpsc::channel(2);
        let (output, mut received) = mpsc::channel(1);
        tx.try_send(event(1)).unwrap();
        tx.try_send(event(2)).unwrap();
        assert!(matches!(
            tx.try_send(event(3)),
            Err(mpsc::error::TrySendError::Full(_))
        ));
        let worker = tokio::spawn(dispatch(
            rpc,
            rx,
            output,
            2,
            Duration::ZERO,
            metrics.clone(),
        ));
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(metrics.current.load(Relaxed), 2);
        tx.try_send(event(3)).unwrap();
        tx.try_send(event(4)).unwrap();
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
        assert_eq!(metrics.current.load(Relaxed), 0);
        assert_eq!(metrics.jobs.load(Relaxed), 0);
        assert_eq!(metrics.cancelled.load(Relaxed), 2);
        assert!(received.recv().await.is_none());
        assert!(tx.is_closed());
    }
    #[tokio::test(start_paused = true)]
    async fn full_output_retains_job_slots_and_cancels_cleanly() {
        let metrics = Arc::new(Metrics::default());
        let rpc = Arc::new(Mock::new(0, FetchError::Transient, Duration::ZERO));
        let (tx, rx) = mpsc::channel(8);
        let (output, received) = mpsc::channel(1);
        for id in 1..=8 {
            tx.send(event(id)).await.unwrap();
        }
        let worker = tokio::spawn(dispatch(
            rpc,
            rx,
            output,
            2,
            Duration::ZERO,
            metrics.clone(),
        ));
        for _ in 0..10 {
            tokio::time::advance(Duration::from_millis(1)).await;
        }
        assert_eq!(metrics.submitted.load(Relaxed), 3); // one delivered + two blocked sends
        assert_eq!(metrics.jobs.load(Relaxed), 2);
        drop(received);
        worker.await.unwrap();
        assert_eq!(metrics.jobs.load(Relaxed), 0);
        assert_eq!(metrics.cancelled.load(Relaxed), 2);
    }
    #[tokio::test(start_paused = true)]
    async fn retry_exhaustion_rate_limits_and_permanent_failures() {
        for (error, attempts) in [
            (FetchError::RateLimited(Duration::from_secs(2)), 5),
            (FetchError::Permanent, 1),
        ] {
            let metrics = Arc::new(Metrics::default());
            let rpc = Arc::new(Mock::new(usize::MAX, error, Duration::ZERO));
            let (tx, rx) = mpsc::channel(1);
            let (output, mut received) = mpsc::channel(1);
            tx.send(event(1)).await.unwrap();
            drop(tx);
            let start = Instant::now();
            dispatch(rpc, rx, output, 1, Duration::ZERO, metrics.clone()).await;
            assert!(received.recv().await.is_none());
            assert_eq!(metrics.attempts.load(Relaxed), attempts);
            assert_eq!(metrics.retries.load(Relaxed), attempts - 1);
            assert_eq!(metrics.failures.load(Relaxed), 1);
            if attempts == 5 {
                assert_eq!(metrics.rate_limits.load(Relaxed), 5);
                assert!(start.elapsed() >= Duration::from_secs(10));
            }
        }
    }
    #[tokio::test(start_paused = true)]
    async fn shared_gate_spaces_requests_and_honors_extended_cooldown() {
        let gate = Arc::new(RequestGate::new(Duration::from_millis(100)));
        gate.acquire().await;
        let start = Instant::now();
        gate.acquire().await;
        assert_eq!(start.elapsed(), Duration::from_millis(100));
        gate.cooldown(Duration::from_secs(5)).await;
        let other = gate.clone();
        let waiter = tokio::spawn(async move {
            other.acquire().await;
            Instant::now()
        });
        tokio::time::advance(Duration::from_secs(1)).await;
        gate.cooldown(Duration::from_secs(8)).await;
        let at = waiter.await.unwrap();
        assert!(at.duration_since(start) >= Duration::from_millis(9100));
    }
}
