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
    payers: HashSet<String>,
    missing_payers: u64,
}
struct Pool {
    info: NewPoolInfo,
    signature: String,
    slot: u64,
    block_time: Option<i64>,
    detected_at: String,
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
}
impl Tracker {
    pub fn register(
        &mut self,
        info: NewPoolInfo,
        signature: &str,
        slot: u64,
        block_time: Option<i64>,
        detected_at: &str,
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
            } else {
                counts.clmm += swaps;
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
            }
        }
        self.pools.retain(|_, p| p.next < 3);
        snapshots
    }
    pub fn shutdown(&mut self, now: f64) -> Vec<Value> {
        let mut result = self.advance(now);
        for pool in self.pools.values() {
            result.push(pool.snapshot(pool.next, (now - pool.start).max(0.0), false));
        }
        self.pools.clear();
        result
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
            "schema_version": 13, "protocol": self.info.protocol, "pool_state": self.info.pool_state,
            "token_mint_0": self.info.token_mint_0, "token_mint_1": self.info.token_mint_1,
            "creation_signature": self.signature, "creation_slot": self.slot,
            "creation_timestamp": self.block_time, "detected_at": self.detected_at,
            "time_basis": "local_notification_receipt", "window_seconds": ENDS[i],
            "interval_start_seconds": STARTS[i], "interval_end_seconds": elapsed,
            "complete": complete, "end_reason": if complete { "window_elapsed" } else { "shutdown" },
            "tx_count": counts.tx, "raydium_invocation_count": counts.invocations,
            "swap_count": counts.swaps, "cpmm_swap_count": counts.cpmm, "clmm_swap_count": counts.clmm,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raydium::InstructionRecord;
    fn pool(address: &str) -> NewPoolInfo {
        NewPoolInfo {
            protocol: "raydium_cpmm",
            instruction: "initialize",
            pool_state: address.into(),
            token_mint_0: "mint0".into(),
            token_mint_1: "mint1".into(),
        }
    }
    fn record(address: &str, swap: bool) -> PoolInstruction {
        PoolInstruction {
            record: InstructionRecord {
                protocol: "raydium_cpmm",
                name: "synthetic".into(),
                discriminator: "".into(),
                known: true,
            },
            accounts: vec![address.into()],
            swap_pool: swap.then(|| address.into()),
        }
    }
    fn register(t: &mut Tracker, address: &str, now: f64) -> bool {
        t.register(pool(address), "creation", 1, Some(123), "test", now)
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
}
