use crate::{config::Config, dedup::remember_signature, listener::LogEvent};
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{CommitmentConfig, RpcTransactionConfig, UiTransactionEncoding},
};
use solana_sdk::signature::Signature;
use std::{
    collections::{HashSet, VecDeque},
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::{
    sync::mpsc,
    time::{sleep, timeout, Duration},
};

pub struct FetchedEvent {
    pub log: LogEvent,
    pub transaction: serde_json::Value,
    pub block_time: Option<i64>,
    pub slot: u64,
}
pub static FETCH_FAILURES: AtomicU64 = AtomicU64::new(0);

pub async fn run(
    config: Config,
    mut input: mpsc::Receiver<LogEvent>,
    output: mpsc::Sender<FetchedEvent>,
) {
    let rpc = RpcClient::new_with_timeout(config.http_url, Duration::from_secs(10));
    let (mut seen, mut queue) = (HashSet::new(), VecDeque::new());
    while let Some(log) = input.recv().await {
        if !remember_signature(&log.signature, &mut seen, &mut queue) {
            continue;
        }
        let Ok(signature) = Signature::from_str(&log.signature) else {
            continue;
        };
        let mut fetched = None;
        for attempt in 0..5 {
            // Single worker + request spacing limits public RPC pressure; timeout also
            // bounds SDK-internal 429 retries. Errors are deliberately URL-redacted.
            sleep(Duration::from_millis(250)).await;
            let result = timeout(
                Duration::from_secs(12),
                rpc.get_transaction_with_config(
                    &signature,
                    RpcTransactionConfig {
                        encoding: Some(UiTransactionEncoding::JsonParsed),
                        commitment: Some(CommitmentConfig::confirmed()),
                        max_supported_transaction_version: Some(0),
                    },
                ),
            )
            .await;
            if let Ok(Ok(tx)) = result {
                fetched = Some(tx);
                break;
            }
            if attempt < 4 {
                sleep(Duration::from_secs(1 << attempt)).await;
            }
        }
        let Some(tx) = fetched else {
            FETCH_FAILURES.fetch_add(1, Ordering::Relaxed);
            eprintln!("RPC fetch exhausted retries; transaction omitted (coverage gap)");
            continue;
        };
        let Ok(transaction) = serde_json::to_value(&tx.transaction) else {
            continue;
        };
        // A confirmed successful log alone is insufficient if the fetched fork differs.
        if transaction.pointer("/meta/err") != Some(&serde_json::Value::Null) {
            continue;
        }
        if output
            .send(FetchedEvent {
                log,
                transaction,
                block_time: tx.block_time,
                slot: tx.slot,
            })
            .await
            .is_err()
        {
            return;
        }
    }
}
