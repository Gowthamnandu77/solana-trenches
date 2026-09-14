mod config;
mod dedup;
mod events;
mod fetcher;
mod freshness;
mod listener;
mod metrics;
mod momentum;
mod persistence;
mod raydium;
mod rpc;

use chrono::Utc;
use config::Config;
use fetcher::FetchedEvent;
use metrics::Metrics;
use momentum::Tracker;
use persistence::{Persistence, Stream};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{atomic::Ordering, Arc},
};
use tokio::{
    sync::mpsc,
    task::JoinSet,
    time::{interval, Duration, Instant, MissedTickBehavior},
};

type Error = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let config = config::load_config()?;
    println!("Solana Trenches — Multi-Protocol Solana Launch Tracker V14 (READ ONLY)");
    println!("Cluster: {}", config.cluster);
    // Manifest-relative path is stable whether run from the workspace or ingestion.
    let mut files = Persistence::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("data")).await?;
    let metrics = Arc::new(Metrics::default());
    println!(
        "Fetch concurrency={} request spacing={}ms",
        config.max_fetch_concurrency, config.rpc_request_interval_ms
    );
    let (sender, receiver) = mpsc::channel(config.input_queue_capacity);
    let (fetched_sender, mut fetched_receiver) = mpsc::channel(128);
    let mut tasks = JoinSet::new();
    for (program, label) in [
        (config.cpmm_program, "raydium_cpmm"),
        (config.clmm_program, "raydium_clmm"),
    ] {
        tasks.spawn(listener::run_listener(
            config.ws_url.clone(),
            program,
            label,
            sender.clone(),
            metrics.clone(),
        ));
    }
    if let Some(program) = config.launchlab_program {
        tasks.spawn(listener::run_listener(
            config.ws_url.clone(),
            program,
            "raydium_launchlab",
            sender.clone(),
            metrics.clone(),
        ));
    }
    drop(sender);
    tasks.spawn(fetcher::run(
        config.clone(),
        receiver,
        fetched_sender,
        metrics.clone(),
    ));
    let start = Instant::now();
    let mut tracker = Tracker::default();
    let mut timer = interval(Duration::from_millis(100));
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut report = interval(Duration::from_secs(10));
    report.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let result: Result<(), Error> = async {
        loop {
            tokio::select! {
                biased;
                signal = tokio::signal::ctrl_c() => { signal?; println!("Ctrl+C: stopping intake and fetches"); break; }
                exited = tasks.join_next() => { return Err(format!("ingestion task stopped unexpectedly: {}", exited.is_some()).into()); }
                _ = report.tick() => {
                    report_metrics(&mut files, &metrics, start.elapsed().as_secs_f64()).await?;
                }
                _ = timer.tick() => {
                    snapshots(&mut files, tracker.advance(start.elapsed().as_secs_f64())).await?;
                }
                event = fetched_receiver.recv() => {
                    let Some(event) = event else { break; };
                    let now = start.elapsed().as_secs_f64();
                    snapshots(&mut files, tracker.advance(now)).await?;
                    process(&config, event, &mut tracker, &mut files, now, &metrics).await?;
                }
            }
        }
        Ok(())
    }.await;
    // Cancelling tasks drops sockets and pending RPC futures. Queued work is
    // deliberately discarded; persisted JSON lines are never cancelled mid-write.
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let final_snapshots =
        snapshots(&mut files, tracker.shutdown(start.elapsed().as_secs_f64())).await;
    let final_metrics = report_metrics(&mut files, &metrics, start.elapsed().as_secs_f64()).await;
    let flushed = files.flush().await;
    println!(
        "Stopped; dropped_logs={} reconnects={} fetch_failures={} rejected_pools={}",
        listener::DROPPED_LOGS.load(Ordering::Relaxed),
        listener::RECONNECTS.load(Ordering::Relaxed),
        fetcher::FETCH_FAILURES.load(Ordering::Relaxed),
        tracker.rejected_pools
    );
    result?;
    final_snapshots?;
    final_metrics?;
    flushed?;
    Ok(())
}

async fn snapshots(files: &mut Persistence, values: Vec<Value>) -> Result<(), Error> {
    for mut value in values {
        value["timestamp"] = json!(Utc::now().to_rfc3339());
        value["scanner_dropped_logs_total"] = json!(listener::DROPPED_LOGS.load(Ordering::Relaxed));
        value["scanner_reconnects_total"] = json!(listener::RECONNECTS.load(Ordering::Relaxed));
        value["scanner_fetch_failures_total"] =
            json!(fetcher::FETCH_FAILURES.load(Ordering::Relaxed));
        files.write(Stream::Momentum, &value).await?;
    }
    Ok(())
}

async fn process(
    config: &Config,
    fetched: FetchedEvent,
    tracker: &mut Tracker,
    files: &mut Persistence,
    now: f64,
    metrics: &Metrics,
) -> Result<(), Error> {
    let decode_start = Instant::now();
    let log = &fetched.log;
    let observed_now = now - log.received_at.elapsed().as_secs_f64();
    let local_age_ms = log.received_at.elapsed().as_millis() as u64;
    let block_age_ms = fetched.block_time.map(|t| {
        Utc::now()
            .timestamp_millis()
            .saturating_sub(t.saturating_mul(1000))
            .max(0) as u64
    });
    let stale_reason =
        freshness::stale_reason(local_age_ms, block_age_ms, config.max_momentum_event_age_ms);
    let stale = stale_reason.is_some();
    metrics.processing_lag.observe(local_age_ms);
    if stale {
        metrics
            .stale_before_momentum
            .fetch_add(1, Ordering::Relaxed);
    }
    let records = raydium::collect_instruction_records(
        &fetched.transaction,
        config.cpmm_program,
        config.clmm_program,
        config.launchlab_program,
    );
    let pools = raydium::detect_new_pools(
        &fetched.transaction,
        config.cpmm_program,
        config.clmm_program,
        config.launchlab_program,
    );
    metrics
        .decode_latency
        .observe(decode_start.elapsed().as_millis() as u64);
    let payer = raydium::fee_payer(&fetched.transaction);
    let source_invoked = raydium::program_invoked_in_logs(&log.logs, log.program_id);
    let status = if records.iter().any(|r| r.known) {
        "decoded"
    } else if source_invoked || !records.is_empty() {
        "invoked_but_not_decoded"
    } else {
        "mentioned_only"
    };
    metrics.processed.fetch_add(1, Ordering::Relaxed);
    if status == "decoded" {
        metrics.decoded.fetch_add(1, Ordering::Relaxed);
    }
    if status == "mentioned_only" {
        metrics.mentioned_only.fetch_add(1, Ordering::Relaxed);
    }
    metrics.unknown.fetch_add(
        records.iter().filter(|r| !r.known).count() as u64,
        Ordering::Relaxed,
    );
    let names: Vec<_> = records.iter().map(|r| r.name.as_str()).collect();
    let timestamp = Utc::now().to_rfc3339();
    for record in records.iter().filter(|r| !r.known) {
        files.write(Stream::Unknown, &json!({
            "schema_version": 14, "timestamp": timestamp, "cluster": config.cluster,
            "slot": fetched.slot, "signature": log.signature, "source": log.source,
            "protocol": record.protocol, "name": record.name, "discriminator": record.discriminator,
            "source_invoked": source_invoked
        })).await?;
    }
    for record in records.iter().filter(|r| r.known) {
        let pool = pools.iter().find(|p| {
            p.protocol == record.protocol
                && matches!(record.event_type, "launch_created" | "pool_created")
        });
        let event = events::LaunchEvent::from_record(
            record,
            pool,
            events::EventContext {
                signature: log.signature.clone(),
                slot: fetched.slot,
                notification_slot: log.slot,
                block_time: fetched.block_time,
                detected_at: timestamp.clone(),
                processing_lag_ms: local_age_ms,
                source_program: log.program_id,
                notification_received_unix_ms: log.received_unix_ms,
                fetch_started_unix_ms: fetched.fetch_started_unix_ms,
                fetch_completed_unix_ms: fetched.fetch_completed_unix_ms,
                source_invoke_count: raydium::program_invoke_count(&log.logs, log.program_id),
            },
        );
        files
            .write(Stream::Events, &event.to_value(&config.cluster))
            .await?;
    }
    for pool in pools {
        let registered = !stale
            && tracker.register(
                pool.clone(),
                &log.signature,
                fetched.slot,
                fetched.block_time,
                &timestamp,
                observed_now,
            );
        files.write(Stream::Pools, &json!({
            "schema_version": 14, "detected_at": timestamp, "cluster": config.cluster,
            "block_time": fetched.block_time, "slot": fetched.slot, "signature": log.signature,
            "protocol": pool.protocol, "instruction": pool.instruction, "pool_state": pool.pool_state,
            "token_mint_0": pool.token_mint_0, "token_mint_1": pool.token_mint_1,
            "creator": pool.creator,
            "tracker_registered": registered, "tracker_rejected_pools_total": tracker.rejected_pools
        })).await?;
        println!(
            "NEW POOL {} {} tracked={registered}",
            pool.protocol, pool.pool_state
        );
    }
    let momentum_start = Instant::now();
    if !stale {
        tracker.observe(&log.signature, payer.as_deref(), &records, observed_now);
    }
    metrics
        .momentum_latency
        .observe(momentum_start.elapsed().as_millis() as u64);
    println!(
        "[{}] status={status} instructions={names:?} multi_route={}",
        log.source,
        records.iter().any(|r| r.protocol == "raydium_cpmm")
            && records.iter().any(|r| r.protocol == "raydium_clmm")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stale_creation_persists_but_never_starts_momentum() {
        let dir = std::env::temp_dir().join(format!("trenches-stale-{}", std::process::id()));
        let mut files = Persistence::open(&dir).await.unwrap();
        let config = Config {
            max_fetch_concurrency: 1,
            input_queue_capacity: 2,
            rpc_request_interval_ms: 100,
            max_fetch_start_age_ms: 5000,
            max_momentum_event_age_ms: 5000,
            cluster: "synthetic".into(),
            http_url: String::new(),
            ws_url: String::new(),
            cpmm_program: "cp",
            clmm_program: "cl",
            launchlab_program: None,
        };
        let transaction = json!({"transaction":{"message":{"instructions":[{
            "programId":"cl", "data":bs58::encode([233,146,209,142,207,104,64,188]).into_string(),
            "accounts":["payer","config","pool","m0","m1"]}]}}});
        let log = listener::LogEvent {
            source: "raydium_clmm",
            program_id: "cl",
            signature: "stale".into(),
            slot: 1,
            logs: vec![],
            received_at: Instant::now(),
            received_unix_ms: 0,
        };
        let mut tracker = Tracker::default();
        let metrics = Metrics::default();
        process(
            &config,
            FetchedEvent {
                log,
                transaction,
                block_time: Some(123),
                slot: 1,
                fetch_started_unix_ms: 0,
                fetch_completed_unix_ms: 0,
            },
            &mut tracker,
            &mut files,
            0.0,
            &metrics,
        )
        .await
        .unwrap();
        assert_eq!(metrics.stale_before_momentum.load(Ordering::Relaxed), 1);
        assert!(tracker.advance(60.0).is_empty());
        files.flush().await.unwrap();
        drop(files);
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
    #[tokio::test]
    async fn synthetic_creation_route_and_snapshots_persist_as_jsonl() {
        let dir = std::env::temp_dir().join(format!("trenches-v12-test-{}", std::process::id()));
        let mut files = Persistence::open(&dir).await.unwrap();
        let config = Config {
            max_fetch_concurrency: 8,
            input_queue_capacity: 2000,
            max_fetch_start_age_ms: 5000,
            max_momentum_event_age_ms: 5000,
            rpc_request_interval_ms: 100,
            cluster: "synthetic".into(),
            http_url: String::new(),
            ws_url: String::new(),
            cpmm_program: "cp",
            clmm_program: "cl",
            launchlab_program: None,
        };
        let ix = |program: &str, disc: [u8; 8], accounts: Vec<&str>| {
            json!({
                "programId": program, "data": bs58::encode(disc).into_string(), "accounts": accounts
            })
        };
        let creation = ix(
            "cl",
            [233, 146, 209, 142, 207, 104, 64, 188],
            vec!["payer", "config", "pool", "mint0", "mint1"],
        );
        let swap = ix(
            "cl",
            [43, 4, 237, 11, 26, 201, 30, 98],
            vec!["payer", "config", "pool"],
        );
        let unknown = ix("cp", [0; 8], vec![]);
        let transaction = json!({"transaction":{"message":{
            "accountKeys":[{"pubkey":"payer","signer":true}], "instructions":[creation, unknown]}},
            "meta":{"err":null,"innerInstructions":[{"instructions":[swap]}]}});
        let log = listener::LogEvent {
            source: "raydium_clmm",
            program_id: "cl",
            signature: "synthetic-signature".into(),
            slot: 42,
            logs: vec!["Program cl invoke [1]".into()],
            received_at: Instant::now(),
            received_unix_ms: 0,
        };
        let mut tracker = Tracker::default();
        process(
            &config,
            FetchedEvent {
                log,
                transaction,
                fetch_started_unix_ms: 0,
                fetch_completed_unix_ms: 0,
                block_time: None,
                slot: 42,
            },
            &mut tracker,
            &mut files,
            0.0,
            &Metrics::default(),
        )
        .await
        .unwrap();
        snapshots(&mut files, tracker.advance(60.0)).await.unwrap();
        files.flush().await.unwrap();
        drop(files);
        for (name, expected) in [
            ("launch_events", 2),
            ("new_launches", 1),
            ("unknown_instructions", 1),
            ("momentum_snapshots", 3),
        ] {
            let data = tokio::fs::read_to_string(dir.join(format!("{name}_v14.jsonl")))
                .await
                .unwrap();
            assert!(data.ends_with('\n'));
            let rows: Vec<Value> = data
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            assert_eq!(rows.len(), expected);
            if name == "momentum_snapshots" {
                assert_eq!(rows[0]["clmm_swap_count"], 1);
                assert_eq!(rows[0]["unique_traders"], 1);
                assert_eq!(rows[0]["creation_slot"], 42);
                assert_eq!(rows[2]["complete"], true);
            }
            if name == "launch_events" {
                assert_eq!(rows[0]["event_type"], "pool_created");
                assert_eq!(rows[0]["protocol"], "raydium_clmm");
            }
        }
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}

async fn report_metrics(
    files: &mut Persistence,
    metrics: &Metrics,
    seconds: f64,
) -> Result<(), Error> {
    let mut value = metrics.snapshot();
    value["timestamp"] = json!(Utc::now().to_rfc3339());
    value["elapsed_seconds"] = json!(seconds);
    value["schema_version"] = json!(14);
    println!("METRICS {value}");
    files.write(Stream::Metrics, &value).await?;
    Ok(())
}
