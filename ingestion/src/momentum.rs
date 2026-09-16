//! Bounded, single-owner local-observation tracker. No network, wall clock, or sleeps.
//! Caller supplies monotonic elapsed seconds, making windows deterministic in tests.
use crate::raydium::{NewPoolInfo, PoolInstruction};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

pub const MAX_POOLS: usize = 256;
const MAX_TRANSACTIONS: usize = 10_000;
const ENDS: [u64; 3] = [10, 30, 60];
const STARTS: [u64; 3] = [0, 10, 30];

#[derive(Default)]
struct Counts {
    tx: u64,
    invocations: u64,
    swaps: u64,
    cpmm: u64,
    clmm: u64,
    launchlab: u64,
    payers: HashSet<String>,
    missing_payers: u64,
}
struct Pool {
    info: NewPoolInfo,
    signature: String,
    slot: u64,
    block_time: Option<i64>,
    detected_at: String,
    detection_lag_ms: u64,
    start: f64,
    next: usize,
    buckets: [Counts; 3],
    signatures: HashSet<String>,
    capped: bool,
}
#[derive(Default)]
pub struct Tracker {
    pools: BTreeMap<String, Pool>,
    pub rejected_pools: u64,
    completed_features: Vec<Value>,
}
impl Tracker {
    #[cfg(test)]
    fn register(
        &mut self,
        info: NewPoolInfo,
        signature: &str,
        slot: u64,
        block_time: Option<i64>,
        detected_at: &str,
        now: f64,
    ) -> bool {
        self.register_with_lag(info, signature, slot, block_time, detected_at, 0, now)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn register_with_lag(
        &mut self,
        info: NewPoolInfo,
        signature: &str,
        slot: u64,
        block_time: Option<i64>,
        detected_at: &str,
        detection_lag_ms: u64,
        now: f64,
    ) -> bool {
        if self.pools.contains_key(&info.pool_state) {
            return false;
        }
        if self.pools.len() >= MAX_POOLS {
            self.rejected_pools += 1;
            return false;
        }
        self.pools.insert(
            info.pool_state.clone(),
            Pool {
                info,
                signature: signature.to_owned(),
                slot,
                block_time,
                detected_at: detected_at.to_owned(),
                detection_lag_ms,
                start: now,
                next: 0,
                buckets: Default::default(),
                signatures: HashSet::new(),
                capped: false,
            },
        );
        true
    }
    /// A transaction counts once per referenced pool. Invocation count is the number
    /// of outer/inner Raydium instructions explicitly referencing that pool, including
    /// unknown instructions. Swap attribution uses the verified positional pool field.
    /// Unique traders count fee payers of *swap* transactions only, not pool creators.
    pub fn observe(
        &mut self,
        signature: &str,
        payer: Option<&str>,
        records: &[PoolInstruction],
        now: f64,
    ) {
        for (address, pool) in &mut self.pools {
            let age = now - pool.start;
            let Some(bucket) = ENDS.iter().position(|end| age >= 0.0 && age < *end as f64) else {
                continue;
            };
            if bucket < pool.next {
                continue;
            }
            let relevant: Vec<_> = records
                .iter()
                .filter(|r| r.protocol == pool.info.protocol && r.accounts.contains(address))
                .collect();
            if relevant.is_empty() || pool.signatures.contains(signature) {
                continue;
            }
            if pool.signatures.len() >= MAX_TRANSACTIONS {
                pool.capped = true;
                continue;
            }
            pool.signatures.insert(signature.to_owned());
            let counts = &mut pool.buckets[bucket];
            counts.tx += 1;
            counts.invocations += relevant.len() as u64;
            let swaps = relevant
                .iter()
                .filter(|r| r.swap_pool.as_ref() == Some(address))
                .count() as u64;
            counts.swaps += swaps;
            if pool.info.protocol == "raydium_cpmm" {
                counts.cpmm += swaps;
            } else if pool.info.protocol == "raydium_clmm" {
                counts.clmm += swaps;
            } else if pool.info.protocol == "raydium_launchlab" {
                counts.launchlab += swaps;
            }
            if swaps > 0 {
                if let Some(payer) = payer {
                    counts.payers.insert(payer.to_owned());
                } else {
                    counts.missing_payers += 1;
                }
            }
        }
    }
    /// Half-open intervals [0,10), [10,30), [30,60). Emit due windows even
    /// during silence, then remove the entire pool state at age >= 60 seconds.
    pub fn advance(&mut self, now: f64) -> Vec<Value> {
        let mut snapshots = Vec::new();
        for pool in self.pools.values_mut() {
            while pool.next < 3 && now - pool.start >= ENDS[pool.next] as f64 {
                snapshots.push(pool.snapshot(pool.next, ENDS[pool.next] as f64, true));
                pool.next += 1;
                if pool.next == 3 {
                    self.completed_features.push(pool.feature(true));
                }
            }
        }
        self.pools.retain(|_, p| p.next < 3);
        snapshots
    }
    pub fn shutdown(&mut self, now: f64) -> Vec<Value> {
        let mut result = self.advance(now);
        for pool in self.pools.values() {
            result.push(pool.snapshot(pool.next, (now - pool.start).max(0.0), false));
            self.completed_features.push(pool.feature(false));
        }
        self.pools.clear();
        result
    }
    pub fn take_features(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.completed_features)
    }
}

/// Activity index, not a profit prediction. Rate terms are normalized against
/// 1 tx/s and 1 swap/s; positive swap-rate change against 1 swap/s. Clamp to 100.
/// No identity, price, volume, or token-accounting assumptions enter this score.
pub fn momentum_score(tx_rate: f64, swap_rate: f64, swap_rate_change: Option<f64>) -> f64 {
    (20.0 * tx_rate + 50.0 * swap_rate + 30.0 * swap_rate_change.unwrap_or(0.0).max(0.0))
        .clamp(0.0, 100.0)
}
impl Pool {
    fn snapshot(&self, i: usize, elapsed: f64, complete: bool) -> Value {
        let counts = &self.buckets[i];
        let duration = elapsed - STARTS[i] as f64;
        let rate = |count: u64| {
            if duration > 0.0 {
                count as f64 / duration
            } else {
                0.0
            }
        };
        let tx_rate = rate(counts.tx);
        let swap_rate = rate(counts.swaps);
        let previous = i.checked_sub(1);
        let tx_change =
            previous.map(|p| tx_rate - self.buckets[p].tx as f64 / (ENDS[p] - STARTS[p]) as f64);
        let swap_change = previous
            .map(|p| swap_rate - self.buckets[p].swaps as f64 / (ENDS[p] - STARTS[p]) as f64);
        // Compare rates at interval midpoints, accounting for unequal window lengths.
        let midpoint_gap =
            previous.map(|p| (STARTS[i] as f64 + elapsed - (STARTS[p] + ENDS[p]) as f64) / 2.0);
        let payers: HashSet<_> = self.buckets[..=i].iter().flat_map(|c| &c.payers).collect();
        json!({
            "schema_version": 15, "protocol": self.info.protocol, "launch_account": self.info.pool_state, "pool_state": self.info.pool_state,
            "token_mint_0": self.info.token_mint_0, "token_mint_1": self.info.token_mint_1,
            "creation_signature": self.signature, "creation_slot": self.slot,
            "creation_timestamp": self.block_time, "detected_at": self.detected_at,
            "time_basis": "local_notification_receipt", "window_seconds": ENDS[i],
            "interval_start_seconds": STARTS[i], "interval_end_seconds": elapsed,
            "complete": complete, "end_reason": if complete { "window_elapsed" } else { "shutdown" },
            "tx_count": counts.tx, "raydium_invocation_count": counts.invocations,
            "swap_count": counts.swaps, "cpmm_swap_count": counts.cpmm, "clmm_swap_count": counts.clmm, "launchlab_swap_count": counts.launchlab,
            "unique_traders": counts.payers.len() as u64, "trader_metric": "distinct_swap_transaction_fee_payers",
            "swap_transactions_without_fee_payer": counts.missing_payers,
            "tx_per_second": tx_rate, "swaps_per_second": swap_rate,
            "tx_rate_change": tx_change, "swap_rate_change": swap_change,
            "tx_acceleration_per_second_squared": tx_change.zip(midpoint_gap).map(|(d,t)| d/t),
            "swap_acceleration_per_second_squared": swap_change.zip(midpoint_gap).map(|(d,t)| d/t),
            "momentum_score": momentum_score(tx_rate, swap_rate, swap_change),
            "cumulative_tx_count": self.buckets[..=i].iter().map(|c| c.tx).sum::<u64>(),
            "cumulative_swap_count": self.buckets[..=i].iter().map(|c| c.swaps).sum::<u64>(),
            "cumulative_unique_traders": payers.len() as u64,
            "counts_capped": self.capped,
            "coverage": "best_effort_confirmed_no_backfill"
        })
    }

    fn feature(&self, complete_window: bool) -> Value {
        let durations = [10.0, 20.0, 30.0];
        let tx: Vec<u64> = self.buckets.iter().map(|c| c.tx).collect();
        let swaps: Vec<u64> = self.buckets.iter().map(|c| c.swaps).collect();
        let rates = |values: &[u64]| -> Vec<f64> {
            values
                .iter()
                .zip(durations)
                .map(|(v, d)| *v as f64 / d)
                .collect()
        };
        let tx_rates = rates(&tx);
        let swap_rates = rates(&swaps);
        let tx_acceleration = vec![
            0.0,
            (tx_rates[1] - tx_rates[0]) / 15.0,
            (tx_rates[2] - tx_rates[1]) / 25.0,
        ];
        let swap_acceleration = vec![
            0.0,
            (swap_rates[1] - swap_rates[0]) / 15.0,
            (swap_rates[2] - swap_rates[1]) / 25.0,
        ];
        let unique: Vec<u64> = (0..3)
            .map(|i| {
                self.buckets[..=i]
                    .iter()
                    .flat_map(|c| &c.payers)
                    .collect::<HashSet<_>>()
                    .len() as u64
            })
            .collect();
        let activity = |f: fn(&Counts) -> u64| -> Vec<u64> { self.buckets.iter().map(f).collect() };
        json!({
            "schema_version": 15,
            "protocol": self.info.protocol,
            "launch_account": self.info.pool_state,
            "base_mint": self.info.token_mint_0,
            "quote_mint": self.info.token_mint_1,
            "creation_signature": self.signature,
            "slot": self.slot,
            "block_time": self.block_time,
            "local_detection_time": self.detected_at,
            "detection_lag_ms": self.detection_lag_ms,
            "tx_counts_10_30_60s": tx,
            "swap_counts_10_30_60s": swaps,
            "approximate_unique_fee_payers_10_30_60s": unique,
            "transaction_rates_10_30_60s": tx_rates,
            "swap_rates_10_30_60s": swap_rates,
            "transaction_acceleration_10_30_60s": tx_acceleration,
            "swap_acceleration_10_30_60s": swap_acceleration,
            "protocol_activity_counts": {
                "raydium_cpmm": activity(|c| c.cpmm),
                "raydium_clmm": activity(|c| c.clmm),
                "raydium_launchlab": activity(|c| c.launchlab)
            },
            "momentum_score": momentum_score(tx_rates[2], swap_rates[2], Some(swap_rates[2] - swap_rates[1])),
            "complete_window": complete_window,
            "scanner_drop_observed": false,
            "stale_event_observed": false,
            "rpc_rate_limited": false,
            "approximate_trader_identity": true,
            "partial_protocol_coverage": false,
            "quality_flags": []
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{features, metrics::Metrics, raydium::InstructionRecord};
    use serde_json::json;
    use std::sync::atomic::Ordering::Relaxed;
    fn pool(address: &str) -> NewPoolInfo {
        NewPoolInfo {
            protocol: "raydium_cpmm",
            instruction: "initialize",
            pool_state: address.into(),
            token_mint_0: "mint0".into(),
            token_mint_1: "mint1".into(),
            creator: None,
        }
    }
    fn record(address: &str, swap: bool) -> PoolInstruction {
        PoolInstruction {
            record: InstructionRecord {
                protocol: "raydium_cpmm",
                name: "synthetic".into(),
                discriminator: "".into(),
                known: true,
                event_type: "swap",
            },
            accounts: vec![address.into()],
            swap_pool: swap.then(|| address.into()),
        }
    }
    fn register(t: &mut Tracker, address: &str, now: f64) -> bool {
        t.register(pool(address), "creation", 1, Some(123), "test", now)
    }

    #[test]
    fn tracks_launchlab_and_clmm_separately() {
        let mut t = Tracker::default();
        t.register(
            NewPoolInfo {
                protocol: "raydium_launchlab",
                instruction: "initialize_v2",
                pool_state: "launch".into(),
                token_mint_0: "base".into(),
                token_mint_1: "quote".into(),
                creator: None,
            },
            "launch-create",
            1,
            None,
            "test",
            0.0,
        );
        t.register(
            NewPoolInfo {
                protocol: "raydium_clmm",
                instruction: "create_pool",
                pool_state: "pool".into(),
                token_mint_0: "base".into(),
                token_mint_1: "quote".into(),
                creator: None,
            },
            "pool-create",
            2,
            None,
            "test",
            0.0,
        );
        let launch_record = PoolInstruction {
            record: InstructionRecord {
                protocol: "raydium_launchlab",
                name: "launchlab_buy_exact_in".into(),
                discriminator: "".into(),
                known: true,
                event_type: "swap",
            },
            accounts: vec!["launch".into()],
            swap_pool: Some("launch".into()),
        };
        let clmm_record = PoolInstruction {
            record: InstructionRecord {
                protocol: "raydium_clmm",
                name: "clmm_swap".into(),
                discriminator: "".into(),
                known: true,
                event_type: "swap",
            },
            accounts: vec!["pool".into()],
            swap_pool: Some("pool".into()),
        };
        t.observe("launch-swap", Some("a"), &[launch_record], 1.0);
        t.observe("clmm-swap", Some("b"), &[clmm_record], 1.0);
        let snapshots = t.advance(10.0);
        assert_eq!(
            snapshots
                .iter()
                .find(|s| s["protocol"] == "raydium_launchlab")
                .unwrap()["launchlab_swap_count"],
            1
        );
        assert_eq!(
            snapshots
                .iter()
                .find(|s| s["protocol"] == "raydium_clmm")
                .unwrap()["clmm_swap_count"],
            1
        );
    }

    #[test]
    fn tracker_emits_each_window_and_final_feature() {
        let mut tracker = Tracker::default();
        assert!(register(&mut tracker, "pool", 0.0));

        // This signature is observed twice but must count only once in [0, 10).
        tracker.observe("first", Some("alice"), &[record("pool", true)], 1.0);
        tracker.observe("first", Some("alice"), &[record("pool", true)], 2.0);
        let ten_seconds = tracker.advance(10.0);
        assert_eq!(ten_seconds.len(), 1);
        assert_eq!(ten_seconds[0]["tx_count"], 1);
        assert_eq!(ten_seconds[0]["swap_count"], 1);
        assert_eq!(ten_seconds[0]["window_seconds"], 10);
        assert_eq!(ten_seconds[0]["complete"], true);
        assert!(tracker.take_features().is_empty());

        tracker.observe("second", Some("bob"), &[record("pool", true)], 10.0);
        let thirty_seconds = tracker.advance(30.0);
        assert_eq!(thirty_seconds[0]["tx_count"], 1);
        assert_eq!(thirty_seconds[0]["window_seconds"], 30);
        assert_eq!(thirty_seconds[0]["complete"], true);
        assert!(tracker.take_features().is_empty());

        tracker.observe("third", Some("carol"), &[record("pool", true)], 30.0);
        let sixty_seconds = tracker.advance(60.0);
        assert_eq!(sixty_seconds[0]["tx_count"], 1);
        assert_eq!(sixty_seconds[0]["window_seconds"], 60);
        assert_eq!(sixty_seconds[0]["complete"], true);

        let features = tracker.take_features();
        assert_eq!(features.len(), 1);
        assert_eq!(features[0]["tx_counts_10_30_60s"], json!([1, 1, 1]));
        assert_eq!(features[0]["swap_counts_10_30_60s"], json!([1, 1, 1]));
        assert_eq!(features[0]["complete_window"], true);
    }

    #[test]
    fn completed_feature_gets_explicit_runtime_quality_flags() {
        let mut tracker = Tracker::default();
        assert!(register(&mut tracker, "pool", 0.0));
        tracker.observe("swap", Some("payer"), &[record("pool", true)], 1.0);
        tracker.advance(60.0);
        let mut feature = tracker.take_features().pop().unwrap();
        assert_eq!(feature["complete_window"], true);

        let metrics = Metrics::default();
        metrics.rate_limits.fetch_add(1, Relaxed);
        features::apply_quality_flags(&mut feature, false, &metrics);

        assert_eq!(feature["approximate_trader_identity"], true);
        assert_eq!(feature["rpc_rate_limited"], true);
        assert_eq!(feature["partial_protocol_coverage"], false);
        assert_eq!(feature["quality_flags"]["complete_window"], true);
        assert_eq!(feature["quality_flags"]["rpc_rate_limited"], true);
        assert_eq!(feature["quality_flags"]["scanner_drop_observed"], false);
    }

    #[test]
    fn windows_dedup_swaps_payers_acceleration_and_score() {
        let mut t = Tracker::default();
        register(&mut t, "a", 0.0);
        t.observe("create", Some("creator"), &[record("a", false)], 0.0);
        t.observe(
            "s1",
            Some("alice"),
            &[record("a", true), record("a", true)],
            9.999,
        );
        t.observe("s1", Some("alice"), &[record("a", true)], 9.999);
        let first = t.advance(10.0).remove(0);
        assert_eq!(first["tx_count"], 2);
        assert_eq!(first["swap_count"], 2);
        assert_eq!(first["raydium_invocation_count"], 3);
        assert_eq!(first["unique_traders"], 1);
        assert_eq!(first["cpmm_swap_count"], 2);
        assert_eq!(first["clmm_swap_count"], 0);
        assert_eq!(first["momentum_score"], 14.0);
        assert!(first["swap_rate_change"].is_null());
        t.observe("s2", Some("alice"), &[record("a", true)], 10.0);
        t.observe("s3", Some("alice"), &[record("a", true)], 20.0);
        let second = t.advance(30.0).remove(0);
        assert_eq!(second["unique_traders"], 1);
        assert_eq!(second["cumulative_unique_traders"], 1);
        assert_eq!(second["swap_rate_change"], -0.1);
        assert!(
            (second["swap_acceleration_per_second_squared"]
                .as_f64()
                .unwrap()
                + 0.1 / 15.0)
                .abs()
                < 1e-12
        );
        assert_eq!(second["momentum_score"], 7.0);
        t.observe("s4", None, &[record("a", true)], 30.0);
        t.observe("late", Some("bob"), &[record("a", true)], 60.0);
        let third = t.advance(60.0).remove(0);
        assert_eq!(third["swap_count"], 1);
        assert_eq!(third["swap_transactions_without_fee_payer"], 1);
        assert!(t.pools.is_empty());
        assert!(t.advance(100.0).is_empty());
    }
    #[test]
    fn simultaneous_pools_silence_and_shutdown() {
        let mut t = Tracker::default();
        register(&mut t, "a", 0.0);
        register(&mut t, "b", 5.0);
        t.observe(
            "route",
            Some("payer"),
            &[record("a", true), record("b", true)],
            6.0,
        );
        assert_eq!(t.advance(60.0).len(), 5);
        assert_eq!(t.pools.len(), 1);
        let last = t.shutdown(62.0).remove(0);
        assert_eq!(last["pool_state"], "b");
        assert_eq!(last["complete"], false);
        assert_eq!(last["interval_end_seconds"], 57.0);
        assert!(t.pools.is_empty());
    }
    #[test]
    fn bounded_capacity_and_no_reset_on_duplicate_creation() {
        let mut t = Tracker::default();
        register(&mut t, "a", 0.0);
        assert!(!register(&mut t, "a", 20.0));
        for i in 1..MAX_POOLS {
            assert!(register(&mut t, &format!("p{i}"), 0.0));
        }
        assert!(!register(&mut t, "overflow", 0.0));
        assert_eq!(t.rejected_pools, 1);
        for i in 0..=MAX_TRANSACTIONS {
            t.observe(&format!("s{i}"), Some("payer"), &[record("a", true)], 1.0);
        }
        let a = t
            .advance(10.0)
            .into_iter()
            .find(|s| s["pool_state"] == "a")
            .unwrap();
        assert_eq!(a["tx_count"], MAX_TRANSACTIONS as u64);
        assert_eq!(a["counts_capped"], true);
        t.advance(60.0);
        assert!(register(&mut t, "new", 60.0));
    }
    #[test]
    fn score_growth_and_clamping() {
        assert_eq!(momentum_score(0.0, 0.0, None), 0.0);
        assert_eq!(momentum_score(0.2, 0.3, Some(0.1)), 22.0);
        assert_eq!(momentum_score(10.0, 10.0, Some(2.0)), 100.0);
    }

    #[test]
    fn completed_feature_contains_stable_windows_and_quality_fields() {
        let mut t = Tracker::default();
        assert!(register(&mut t, "feature", 0.0));
        t.observe("swap-1", Some("payer"), &[record("feature", true)], 1.0);
        let snapshots = t.advance(60.0);
        assert_eq!(snapshots.len(), 3);
        let features = t.take_features();
        assert_eq!(features.len(), 1);
        let feature = &features[0];
        assert_eq!(feature["schema_version"], 15);
        assert_eq!(feature["tx_counts_10_30_60s"].as_array().unwrap().len(), 3);
        assert_eq!(feature["swap_counts_10_30_60s"][0], 1);
        assert_eq!(feature["complete_window"], true);
        assert_eq!(feature["approximate_trader_identity"], true);
        serde_json::to_string(feature).unwrap();
    }
}
