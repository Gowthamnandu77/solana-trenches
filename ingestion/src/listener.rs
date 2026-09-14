use crate::{
    metrics::Metrics,
    queue::{FreshQueue, PushResult},
};
use futures_util::StreamExt;
use solana_client::{
    nonblocking::pubsub_client::PubsubClient,
    rpc_config::{CommitmentConfig, RpcTransactionLogsConfig, RpcTransactionLogsFilter},
};
use std::sync::{atomic::Ordering::Relaxed, Arc};
use tokio::time::{sleep, timeout, Duration, Instant};

#[derive(Clone, Debug)]
pub struct LogEvent {
    pub source: &'static str,
    pub program_id: &'static str,
    pub signature: String,
    pub slot: u64,
    pub logs: Vec<String>,
    pub received_at: Instant,
    pub received_unix_ms: i64,
}

/// The supervisor cancels these tasks on shutdown, including pending connects/sends.
pub async fn run_listener(
    ws_url: String,
    program_id: &'static str,
    label: &'static str,
    sender: Arc<FreshQueue>,
    metrics: Arc<Metrics>,
) {
    let mut backoff = 3;
    loop {
        println!("[{label}] Connecting...");
        let session_start = Instant::now();
        if let Ok(Ok(client)) = timeout(Duration::from_secs(20), PubsubClient::new(&ws_url)).await {
            let subscription = timeout(
                Duration::from_secs(20),
                client.logs_subscribe(
                    RpcTransactionLogsFilter::Mentions(vec![program_id.to_owned()]),
                    RpcTransactionLogsConfig {
                        commitment: Some(CommitmentConfig::confirmed()),
                    },
                ),
            )
            .await;
            if let Ok(Ok((mut stream, unsubscribe))) = subscription {
                println!("[{label}] LISTENER ACTIVE");
                while let Some(response) = stream.next().await {
                    metrics.notifications_received.fetch_add(1, Relaxed);
                    let value = response.value;
                    if value.err.is_some() {
                        metrics.notifications_failed.fetch_add(1, Relaxed);
                        continue;
                    }
                    if !metrics.early_dedup.lock().unwrap().admit(&value.signature) {
                        metrics.duplicates.fetch_add(1, Relaxed);
                        continue;
                    }
                    let event = LogEvent {
                        source: label,
                        program_id,
                        signature: value.signature,
                        slot: response.context.slot,
                        logs: value.logs,
                        received_at: Instant::now(),
                        received_unix_ms: chrono::Utc::now().timestamp_millis(),
                    };
                    // Drop with a visible counter instead of silently stalling the socket forever.
                    let signature = event.signature.clone();
                    let received_at = event.received_at;
                    match sender.push(event) {
                        PushResult::Accepted { evicted } => {
                            let mut queued = metrics.queued.lock().unwrap();
                            queued.insert(signature, received_at);
                            for old in evicted {
                                queued.remove(&old.signature);
                                metrics.stale_evicted_before_queue.fetch_add(1, Relaxed);
                            }
                            metrics.fresh_candidates_admitted.fetch_add(1, Relaxed);
                            metrics
                                .queue_high_water
                                .fetch_max(sender.len() as u64, Relaxed);
                        }
                        PushResult::Closed => return,
                        PushResult::Full => {
                            metrics.notifications_dropped.fetch_add(1, Relaxed);
                            metrics
                                .queue_high_water
                                .fetch_max(sender.len() as u64, Relaxed);
                            DROPPED_LOGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
                let _ = timeout(Duration::from_secs(2), unsubscribe()).await;
            }
        }
        // Never print library errors: they may contain authenticated RPC URLs.
        eprintln!("[{label}] disconnected or unavailable; retry in {backoff}s (coverage gap)");
        RECONNECTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if session_start.elapsed() > Duration::from_secs(60) {
            backoff = 3;
        }
        sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(60);
    }
}
pub static DROPPED_LOGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static RECONNECTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
