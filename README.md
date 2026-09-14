# Solana Trenches

A Rust/Solana engineering and market-intelligence project. The repository contains an Anchor program under `programs/` and a separate read-only ingestion application under `ingestion/`.

## Raydium New Pool Momentum Tracker — V11

Targeted Solana WebSocket subscriptions watch Raydium CPMM and CLMM program mentions. An async Rust pipeline fetches successful confirmed transactions, decodes outer and inner Raydium instructions, detects pool creation, and tracks newly observed pools concurrently. V10 program IDs, discriminator mappings, protocol classifications, unknown-instruction discovery, and route detection are retained.

```text
CPMM logs subscription ─┐
                       ├─ bounded MPSC (2,000) ─ fetch/retry + dedup task
CLMM logs subscription ─┘                              │
                                             bounded MPSC (128)
                                                      │
                                  decoder + pool tracker + 100ms timer
                                                      │
                                 events / new pairs / unknowns / snapshots
                                               append-only JSONL
```

This is a learning and portfolio project, not a production-ready indexer or a profitability claim. It never loads wallets, signs or sends transactions, or trades tokens.

### Run

Use the existing Rust toolchain pinned in `rust-toolchain.toml`. From this repository:

```bash
cd /home/cure/solana-trenches/ingestion
# Set SOLANA_CLUSTER, SOLANA_HTTP_URL and SOLANA_WS_URL in your local .env.
SOLANA_CLUSTER=mainnet cargo run
```

Defaults are Devnet and its public RPC endpoints. `mainnet`, `mainnet-beta`, and `devnet` are accepted. Environment variables override `.env`; dotenv searches the working directory and ancestors. HTTP and WebSocket endpoints must target the same cluster. Endpoint URLs and RPC error payloads are never printed, because URLs can contain credentials. No wallet configuration is needed.

Press **Ctrl+C** to stop intake, cancel socket/RPC tasks, discard queued work, write partial snapshots for active pools, and flush all writers. Partial snapshots have `complete: false` and the actual interval end. A completed line is flushed before handling the next item. This handles ordinary Ctrl+C, not power loss, disk failure, or SIGKILL; there is no fsync durability guarantee.

### Windows and metrics

Observation starts when the creation transaction is decoded locally. A monotonic clock assigns events to half-open intervals **[0,10), [10,30), [30,60)** seconds. Snapshots appear at 10, 30, and 60 seconds, including windows with zero activity. State expires at 60 seconds. `window_seconds` is the scheduled endpoint; `interval_start_seconds` and `interval_end_seconds` identify the actual interval. Counts without a `cumulative_` prefix refer to that interval. Creation itself counts as activity, but not as a swap or unique trader.

Each snapshot includes pool/mint addresses, protocol, creation signature/slot, nullable creation block timestamp (Unix seconds), detection timestamp, transaction count, pool-referencing Raydium instruction invocation count, decoded swap count, CPMM/CLMM swap counts, and rates. Counts use integers; only rates, elapsed durations, and scores use floating point.

`unique_traders` means **distinct fee payers of observed swap transactions**, requiring explicit signer metadata on account zero. It is an approximation: routers, bots, aggregators, sponsored fees, and CPI can separate a fee payer from the economic trader. Missing attribution is counted separately. Cumulative uniqueness is a union across windows, not the sum of their counts.

Swap attribution uses the instruction's pool field: CPMM swap account index 3; CLMM swap/swap_v2 index 2. Transaction and invocation counts include Raydium instructions that explicitly reference the tracked pool account; they do not imply a specific economic action. Unknown instructions can count as activity but never as swaps. Merely mentioning a program in transaction account keys does not count as pool activity.

For each interval:

```text
tx_rate   = interval transaction count / interval duration
swap_rate = interval decoded swap count / interval duration
rate_change = current interval rate - previous interval rate
acceleration = rate_change / distance between interval midpoints
score = clamp(20*tx_rate + 50*swap_rate + 30*max(swap_rate_change, 0), 0, 100)
```

The first window has no previous rate, so changes/accelerations are null and the score's change term is zero. Midpoint distances are 15 seconds and 25 seconds for complete windows. The score is a deterministic activity index with reference rates of 1 transaction/second and 1 swap/second. It does **not** predict returns. Buy/sell direction, volume, USD value, price, and economic trader identity are deliberately omitted.

### Coverage and memory limits

These are **local decoded-observation** metrics, not an exact reconstruction of the first on-chain seconds. HTTP backlog, retries, out-of-order notifications, confirmed-fork changes, reconnect gaps, and dropped messages can omit or shift activity. Events before creation is discovered are not backfilled. `notification_to_processing_seconds` exposes processing lag. `complete` means the local interval elapsed, not that chain coverage is complete. Confirmed transactions can still be rolled back; there is no fork reconciliation.

There are at most 256 active pools, 10,000 retained transaction signatures per pool, 10,000 global dedup signatures, 2,000 queued log events, and 128 queued fetched transactions. Each pool has three fixed buckets. Payer sets are bounded by the per-pool transaction cap. When that cap is hit, further counts stop and `counts_capped` becomes true. New pools beyond capacity remain in pool JSONL with `tracker_registered: false`; an explicit rejection counter is recorded. Global dedup evicts oldest signatures; per-pool dedup prevents recounting within an active tracker even after global eviction. A duplicate creation does not restart an active tracker.

Full input queues drop notifications and increment a counter. Snapshots include scanner-wide cumulative drops, reconnects, and exhausted fetches; those counters cannot identify which particular pool was affected. One fetch worker spaces requests by at least 250ms, retries up to five times with exponential delays, and bounds each request including SDK retries. WebSocket reconnect delay grows from 3 to 60 seconds. This is deliberately conservative for public RPC; it cannot keep up with all Mainnet traffic on every endpoint. Transaction version support remains V10's legacy/v0 setting; unsupported versions are omitted and counted as fetch failures. V1 decoding needs separate SDK compatibility validation.

JSONL files grow on disk and need external retention/rotation for longer runs. Memory is bounded by item counts and chain transaction sizes; this application is not hardened against a malicious RPC serving arbitrarily large responses.

### Output

Paths are anchored to `ingestion/data/` regardless of the launch directory:

- `raydium_events_v11.jsonl`
- `new_pairs_v11.jsonl`
- `unknown_instructions_v11.jsonl`
- `momentum_snapshots_v11.jsonl`

All are append-only, versioned structured JSONL and ignored by Git. Empty pool/snapshot files during a short run are expected if no new pool is observed. Generated trading data, `.env`, wallet/key files, build artifacts, and dependencies should remain local.

### Validate without Mainnet

From `ingestion/`:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build
```

Synthetic tests exercise discriminator preservation, outer/inner routes, pool account extraction, missing signer metadata, dedup eviction, exact time boundaries, swap/payer counts, unequal-window acceleration, score behavior, concurrent pools, expiration, capacity limits, and partial shutdown. No network is needed for those tests.

### Code and learning guide

| File | Responsibility |
| --- | --- |
| `ingestion/src/main.rs` | Supervises tasks, selects timer/events/shutdown, coordinates output |
| `ingestion/src/config.rs` | Loads cluster and private endpoint configuration |
| `ingestion/src/listener.rs` | Two targeted WebSocket subscriptions and reconnect backoff |
| `ingestion/src/fetcher.rs` | Sequential HTTP fetching, retries, successful-meta check |
| `ingestion/src/dedup.rs` | Bounded signature set plus eviction queue |
| `ingestion/src/raydium.rs` | Preserved V10 discriminator decoder plus pool/swap attribution |
| `ingestion/src/momentum.rs` | Bounded pool state, windows, metrics, score, synthetic tests |
| `ingestion/src/persistence.rs` | Async JSONL append and explicit flush |

**Tokio tasks:** each listener and the fetcher spends much of its life waiting for network I/O. `.await` lets Tokio run other ready work instead of blocking a thread. The tracker has one owner in the main task, so no shared mutable pool map or mutex is needed.

**MPSC:** multiple producers send messages to one consumer. Both listeners share a bounded queue feeding the fetcher. A second bounded queue feeds the coordinator. Ownership of each event moves through the channels; capacity prevents an unlimited backlog.

**Subscriptions versus fetching:** `logsSubscribe` asks an RPC server to push notifications for transactions mentioning one program address. Notifications contain signatures and logs, not the full instruction accounts/data required for pool extraction. `getTransaction` fetches that detail separately; retries allow for indexing delay after a notification.

**Discriminators:** the supported Anchor instruction formats begin with eight identifying bytes, conventionally the first eight bytes of SHA-256 of `global:<instruction_name>`. The decoder base58-decodes instruction data and compares those bytes with V10's known constants. Program ID scopes the comparison. Both outer instructions and inner CPI instructions matter for routed swaps.

**Deduplication:** the same signature can arrive through both subscriptions or after reconnection. A hash set provides fast membership checks; a FIFO queue remembers which signature to evict when capacity is reached. Each active pool also retains a bounded signature set.

**State and timers:** a `BTreeMap` holds pool metadata and three count buckets. A Tokio interval wakes the coordinator about every 100ms; `select!` also accepts transactions and Ctrl+C. No task sleeps for 60 seconds holding up ingestion. Elapsed monotonic time determines which windows are due; the tests inject elapsed seconds directly.

**Shutdown:** the supervisor cancels and joins producer tasks, emits unfinished-window snapshots, and flushes files. Writes are awaited outside the cancellation selection, so Ctrl+C does not interrupt a JSONL write halfway through. Queued work is intentionally abandoned for prompt shutdown and is not claimed as observed activity.

**Memory:** unchecked channels, permanent pool maps, ever-growing dedup sets, unique-payer sets, and unknown-discriminator statistics are common growth traps. V11 bounds these retained structures and avoids an unbounded instruction-name statistics map.

Study these five paths first: `main`'s `select!` and cleanup; `run_listener`; `collect_instruction_records`/`classify_instruction`; `Tracker::observe`/`advance`; and `Pool::snapshot`/`momentum_score`.

Account layouts were checked against Raydium's source: [CPMM swap](https://github.com/raydium-io/raydium-cp-swap/blob/master/programs/cp-swap/src/instructions/swap_base_input.rs), [CLMM swap](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap.rs), [CLMM swap v2](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap_v2.rs), [CPMM initialize](https://github.com/raydium-io/raydium-cp-swap/blob/master/programs/cp-swap/src/instructions/initialize.rs), [CLMM create pool](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/create_pool.rs), and [CLMM customizable pool](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/create_customizable_pool.rs).
