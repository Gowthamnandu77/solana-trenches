# Solana Trenches

A Rust/Solana engineering and market-intelligence project. The repository contains an Anchor program under `programs/` and a separate read-only ingestion application under `ingestion/`.

## Raydium New Pool Momentum Tracker — V12

Targeted Solana WebSocket subscriptions watch Raydium CPMM and CLMM program mentions. An async Rust pipeline fetches successful confirmed transactions, decodes outer and inner Raydium instructions, detects pool creation, and tracks newly observed pools concurrently. V10 program IDs, discriminator mappings, protocol classifications, unknown-instruction discovery, and route detection are retained.

```text
CPMM logs subscription ─┐
                       ├─ bounded MPSC (2,000) ─ dedup + bounded concurrent fetch jobs
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

These are **local decoded-observation** metrics, not an exact reconstruction of the first on-chain seconds. HTTP backlog, retries, out-of-order notifications, confirmed-fork changes, reconnect gaps, and dropped messages can omit or shift activity. Events before creation is discovered are not backfilled. Concurrent fetches deliver in completion order, so a fast swap fetch can precede a slower pool-creation fetch. There is no unbounded reorder buffer or historical replay. Routed swaps still use their original per-instruction pool account, never a transaction-wide pool guess. `notification_to_processing_seconds` exposes processing lag. `complete` means the local interval elapsed, not that chain coverage is complete. Confirmed transactions can still be rolled back; there is no fork reconciliation.

There are at most 256 active pools, 10,000 retained transaction signatures per pool, 10,000 global dedup signatures, 2,000 queued log events, and 128 queued fetched transactions. Each pool has three fixed buckets. Payer sets are bounded by the per-pool transaction cap. When that cap is hit, further counts stop and `counts_capped` becomes true. New pools beyond capacity remain in pool JSONL with `tracker_registered: false`; an explicit rejection counter is recorded. Global dedup evicts oldest signatures; per-pool dedup prevents recounting within an active tracker even after global eviction. A duplicate creation does not restart an active tracker.

Full input queues drop notifications and increment a counter. Snapshots include scanner-wide cumulative drops, reconnects, and exhausted fetches; those counters cannot identify which particular pool was affected. A dispatcher runs at most `MAX_FETCH_CONCURRENCY` fetch jobs (default 8, range 1–64). All jobs share a request-start gate (`RPC_REQUEST_INTERVAL_MS`, default 100ms, range 10–10,000ms); concurrency overlaps network latency without multiplying this rate limit. Retries occupy job slots and share the same gate. HTTP 429 and recognized JSON-RPC throttling extend a global cooldown, honoring numeric or HTTP-date Retry-After (clamped to 1–120 seconds). Transient failures retry up to five total attempts with 500ms, 1s, 2s, and 4s waits. Permanent HTTP/client/unsupported-version errors stop immediately. A pooled reqwest transport makes getTransaction requests directly so SDK-internal 429 loops cannot hide attempts. Each request has a 10-second HTTP deadline and a 12-second outer deadline. WebSocket reconnect delay grows from 3 to 60 seconds. This is deliberately conservative for public RPC; it cannot keep up with all Mainnet traffic on every endpoint. Transaction version support remains V10's legacy/v0 setting; unsupported versions are omitted and counted as fetch failures. V1 decoding needs separate SDK compatibility validation.

JSONL files grow on disk and need external retention/rotation for longer runs. Memory is bounded by item counts and chain transaction sizes; this application is not hardened against a malicious RPC serving arbitrarily large responses.

### Output

Paths are anchored to `ingestion/data/` regardless of the launch directory:

- `raydium_events_v12.jsonl`
- `new_pairs_v12.jsonl`
- `unknown_instructions_v12.jsonl`
- `momentum_snapshots_v12.jsonl`
- `runtime_metrics_v12.jsonl` (every 10 seconds and at shutdown)

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
| `ingestion/src/fetcher.rs` | Bounded concurrent jobs, dispatch dedup, shared request gate and retries |
| `ingestion/src/rpc.rs` | Read-only HTTP transport, explicit status/Retry-After handling |
| `ingestion/src/metrics.rs` | Per-run counters and cancellation-safe concurrency gauges |
| `ingestion/src/dedup.rs` | Bounded signature set plus eviction queue |
| `ingestion/src/raydium.rs` | Preserved V10 discriminator decoder plus pool/swap attribution |
| `ingestion/src/momentum.rs` | Bounded pool state, windows, metrics, score, synthetic tests |
| `ingestion/src/persistence.rs` | Async JSONL append and explicit flush |

**Tokio tasks:** each listener and the fetcher spends much of its life waiting for network I/O. `.await` lets Tokio run other ready work instead of blocking a thread. The tracker has one owner in the main task, so no shared mutable pool map or mutex is needed.

**MPSC:** multiple producers send messages to one consumer. Both listeners share a bounded queue feeding the fetcher. A second bounded queue feeds the coordinator. `FuturesUnordered` polls up to the configured number of jobs concurrently inside one dispatcher task; there is no task spawned per notification and no unlimited semaphore wait queue. A full output queue holds job slots, propagating backpressure. Ownership of each event moves through the channels; capacity prevents an unlimited backlog.

**Subscriptions versus fetching:** `logsSubscribe` asks an RPC server to push notifications for transactions mentioning one program address. Notifications contain signatures and logs, not the full instruction accounts/data required for pool extraction. `getTransaction` fetches that detail separately; retries allow for indexing delay after a notification.

**Discriminators:** the supported Anchor instruction formats begin with eight identifying bytes, conventionally the first eight bytes of SHA-256 of `global:<instruction_name>`. The decoder base58-decodes instruction data and compares those bytes with V10's known constants. Program ID scopes the comparison. Both outer instructions and inner CPI instructions matter for routed swaps.

**Deduplication:** the same signature can arrive through both subscriptions or after reconnection. A hash set provides fast membership checks; a FIFO queue remembers which signature to evict when capacity is reached. Each active pool also retains a bounded signature set. The dispatcher separately retains pending signatures (at most the job limit), so eviction from the 10,000-entry history cannot admit a second copy of an in-flight job. Retries emit a result only once, after success. Completed signatures can be fetched again after bounded history eviction; per-pool dedup still prevents a second momentum update during that pool’s observation period.

**State and timers:** a `BTreeMap` holds pool metadata and three count buckets. A Tokio interval wakes the coordinator about every 100ms; `select!` also accepts transactions and Ctrl+C. No task sleeps for 60 seconds holding up ingestion. Elapsed monotonic time determines which windows are due; the tests inject elapsed seconds directly.

**Shutdown:** the supervisor cancels and joins producer tasks, emits unfinished-window snapshots, and flushes files. Writes are awaited outside the cancellation selection, so Ctrl+C does not interrupt a JSONL write halfway through. Queued work is intentionally abandoned for prompt shutdown and is not claimed as observed activity.

**Memory:** unchecked channels, permanent pool maps, ever-growing dedup sets, unique-payer sets, and unknown-discriminator statistics are common growth traps. V12 bounds these retained structures and avoids an unbounded instruction-name statistics map.

Study these five paths first: `main`'s `select!` and cleanup; `run_listener`; `collect_instruction_records`/`classify_instruction`; `Tracker::observe`/`advance`; and `Pool::snapshot`/`momentum_score`.

Account layouts were checked against Raydium's source: [CPMM swap](https://github.com/raydium-io/raydium-cp-swap/blob/master/programs/cp-swap/src/instructions/swap_base_input.rs), [CLMM swap](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap.rs), [CLMM swap v2](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap_v2.rs), [CPMM initialize](https://github.com/raydium-io/raydium-cp-swap/blob/master/programs/cp-swap/src/instructions/initialize.rs), [CLMM create pool](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/create_pool.rs), and [CLMM customizable pool](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/create_customizable_pool.rs).


## V12 throughput diagnosis and runtime metrics

V11 had a single sequential dequeue/fetch loop. Its fixed 250ms wait alone capped service below 4 transactions/second; network time and retries reduced that further. When arrival rate exceeded service rate, the 2,000-notification queue filled and `try_send` explicitly dropped notifications. Duplicate notifications consumed queue slots before dispatch deduplication. The 45-second baseline processed 31 transactions (0.69/s), including 12 decoded and one multi-protocol route, and dropped 110 notifications. It had no request-latency/attempt counters, so the historical split between HTTP latency, retry delay, and SDK-internal throttling cannot be reconstructed.

V12 keeps the same 2,000/128 queue capacities and adds bounded concurrency, not a larger backlog. Defaults permit at most 10 request starts/second with up to 8 jobs, including retry waits and blocked result delivery. Tune the rate to your provider's allowance; raising concurrency alone does not bypass the shared gate. Add to your existing untracked `.env`:

```dotenv
MAX_FETCH_CONCURRENCY=8
RPC_REQUEST_INTERVAL_MS=100
```

`runtime_metrics_v12.jsonl` and console `METRICS` lines report notification receipt (including failed transactions), failed-notification filtering, drops, approximate queue high-water marks, unique submissions, duplicates filtered, successful fetches, actual retry attempts, terminal failures, detected throttling, decoded/mentioned-only transaction counts, unknown instruction counts, processed transactions, RPC attempts, current/peak HTTP concurrency, active jobs, cancelled jobs, and mean HTTP-attempt latency. Latency includes body parsing/timeouts/cancellation but excludes queue residence, gate waits and retry sleeps. Event `notification_to_processing_seconds` includes the backlog. Gauges/counters are concurrent approximations, not an atomic accounting ledger. Success counts successful fetches before output delivery; shutdown may discard fetched or queued work. Metrics reset per process; timestamps and elapsed seconds distinguish runs in the append-only file.

HTTP attempts and job counts differ: a job waiting for rate permission or retry backoff occupies capacity but has no active HTTP request. RAII guards decrement both gauges during cancellation. Aborting the dispatcher drops its owned futures, so no detached RPC workers survive shutdown. Existing timer windows, pool expiration, decoder mappings, and routed-pool attribution are preserved.

Synthetic V12 tests cover concurrency bounds, duplicate inputs through transient retries, retry exhaustion, permanent errors, shared rate-gate spacing/cooldown extension, full input/output queues, and shutdown while requests or result sends are pending. All original V11 tests remain.

### Mainnet benchmark (2026-09-14)

Read-only, configured endpoint, default 8 jobs / 100ms shared request spacing, unchanged queue capacities. Both CPMM and CLMM listeners activated; SIGINT stopped the scanner at 60 seconds with exit code 0 and flushed valid JSONL. No new pool appeared in this sample; synthetic tests cover pool/snapshot persistence.

| Measurement | V11 baseline | V12 |
| --- | ---: | ---: |
| Duration | 45s | 60s |
| Processed transactions | 31 | 45 |
| Processed per second | 0.69 | 0.75 |
| Decoded transactions | 12 | 33 |
| Mentioned-only transactions | 19 | 12 |
| Multi-protocol routes | 1 | 3 |
| Unknown instructions | 0 | 0 |
| Notifications dropped | 110 | 0 |

V12 received 3,140 notifications, filtered 1,196 failed-transaction notifications, submitted 53 unique transactions, and filtered three duplicate notifications at dispatch. It made 52 HTTP attempts: 45 successful fetches and seven detected rate limits, including six retry attempts. No jobs exhausted retries. Mean HTTP-attempt latency was 210ms; peak actual HTTP concurrency was seven (configured job cap eight). Input/output queue high-water marks were 1,888/2,000 and 1/128. Shutdown cancelled eight pending jobs; current concurrency and active jobs returned to zero. All 45 event lines parsed as JSON.

**Remaining bottleneck:** the configured RPC's rate limit dominates. The shared cooldown was active much of the run; the result queue stayed nearly empty while the input backlog grew. Mean notification-to-processing lag was 22.7 seconds and the last processed event lagged by 53.6 seconds. Zero drops in this sample is not sustainable coverage: the input queue was nearly full and queued notifications were discarded at shutdown. More concurrency cannot overcome this provider allowance; sustainable coverage requires an endpoint/rate budget that can serve the incoming unique transaction rate, or a separately designed narrower intake policy. This comparison used different live traffic at different times; the observed roughly 9% throughput increase is not a controlled speedup claim.

## V13 freshness controls

V13 preserves V12 decoding and adds shared listener deduplication before the bounded
queue. Failed notifications are filtered before admission; no log-content filter
is used. Local monotonic notification receipt defines momentum time, with half-open
intervals [0,10), [10,30), [30,60). Late arrivals cannot rewrite emitted intervals.
These are scanner observation windows, not exact on-chain first-second activity.

`MAX_FETCH_START_AGE_MS=5000` skips work older than five seconds after the shared
request gate, including retries. `MAX_MOMENTUM_EVENT_AGE_MS=5000` rejects momentum
updates if local age or available block-time age exceeds five seconds. Decoded
stale transactions remain persisted with a reason; stale-before-fetch work has a
counter and no transaction body. Block time is second-resolution chain metadata;
notification and processing timestamps are separate local wall-clock measurements.
Monotonic clocks determine local age, so wall-clock corrections cannot change queue age.
Missing block time cannot establish chain freshness.

`FETCH_CONCURRENCY=8`, `RPC_MIN_REQUEST_INTERVAL_MS=100`, and
`INPUT_QUEUE_CAPACITY=2000` control bounded resource usage. The previous
`MAX_FETCH_CONCURRENCY` and `RPC_REQUEST_INTERVAL_MS` names remain fallback aliases.
`SOLANA_WS_URL` and `SOLANA_HTTP_URL` independently configure intake and transaction
fetching. URLs and provider error bodies are never printed. Defaults use public
cluster endpoints; a dedicated RPC can improve coverage without changing the pipeline.

Metrics include queue occupancy, oldest queued age, high-water, queue wait,
fetch-start lag, processing lag, decoder and momentum durations, and stale counts.
Latency histograms use 32 fixed logarithmic buckets: p50/p95 are approximate bucket
upper bounds, max is measured. Memory does not grow with runtime.

Root Cargo commands default to the `ingestion` package. The preserved Anchor counter
example remains a separate workspace member; its LiteSVM integration test requires
`target/deploy/solana_trenches.so` from an SBF build and is not part of scanner tests.
Workspace formatting also normalized three existing counter-example files.

### V13 Mainnet benchmark (2026-09-14)

Read-only public Mainnet endpoint, 60 seconds, with the default V13 settings:
fetch concurrency 8, 100ms minimum RPC spacing, and a 2,000-entry input queue.
Both CPMM and CLMM listeners were active; the scanner exited cleanly after SIGINT
and flushed valid JSONL.

| Measurement | V13 |
| --- | ---: |
| Duration | 60.03s |
| Processed transactions | 10 |
| Decoded transactions | 3 |
| Mentioned-only transactions | 7 |
| Notifications received | 3,251 |
| Failed notifications filtered | 1,285 |
| Duplicates filtered | 230 |
| Stale before fetch | 1,536 |
| Stale before momentum | 10 |
| Successful fetches / RPC attempts | 10 / 11 |
| Rate limits / retries | 1 / 0 |
| Notifications dropped | 0 |
| Peak HTTP concurrency | 3 |
| Input queue high-water | 296 |
| Mean HTTP-attempt latency | 154.8ms |

No new pool appeared in this sample. The final record reported eight cancelled
jobs during shutdown; current jobs and active HTTP concurrency returned to zero.
