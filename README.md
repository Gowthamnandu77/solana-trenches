# Solana Trenches

A Rust/Solana engineering and market-intelligence project. The repository contains an Anchor program under `programs/` and a separate read-only ingestion application under `ingestion/`.

## Solana Launch Intelligence Engine — V15

Targeted Solana WebSocket subscriptions feed verified Raydium decoders and a normalized launch event model. The read-only Rust pipeline applies freshness rules, tracks each launch or pool through 10/30/60-second windows, emits explainable activity features, and preserves quality flags for later research. It is an engineering and data-product project, not a trading system.

```text
CPMM / CLMM / LaunchLab logs
              │ bounded intake queue + signature deduplication
              ▼
      bounded fetch jobs ─ shared request pacing ─ retry/cooldown
              │
              ▼
 verified outer/inner decoding ─ freshness filter ─ normalized LaunchEvent
              │                                      │
              ▼                                      ▼
     per-launch windows ──────────────── append-only V15 JSONL dataset
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

- `launch_events_v15.jsonl`
- `new_launches_v15.jsonl`
- `unknown_instructions_v15.jsonl`
- `momentum_snapshots_v15.jsonl`
- `features_v15.jsonl`
- `runtime_metrics_v15.jsonl` (every 10 seconds and at shutdown)

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

## V15 feature dataset

V15 keeps the verified Raydium CPMM, CLMM, and Mainnet LaunchLab coverage from
V14. It emits one `features_v15.jsonl` record per launch or pool when its
60-second lifecycle completes, or a partial record during graceful shutdown.
The record contains protocol, launch account, mints, creation metadata,
10/30/60-second transaction and swap counts, cumulative approximate fee-payer
sets, rates, acceleration, protocol activity counts, the deterministic score,
and scanner quality indicators. Missing or uncertain values are retained as
null or explicitly flagged; the scanner does not infer price, liquidity, USD
value, PnL, buy pressure, bot identity, or profitability.

The score is an activity index:

```text
score = clamp(20 * tx_rate + 50 * swap_rate
              + 30 * max(swap_rate - previous_swap_rate, 0), 0, 100)
```

Rates are per second for the relevant window and are normalized against one
transaction/second and one swap/second. Acceleration is the rate change divided
by the midpoint distance between adjacent windows (15 seconds, then 25
seconds). The score is deterministic, bounded, and explainable. It measures
observed local activity; it is not a price, liquidity, trading, or return
prediction.

Every feature includes `quality_flags`: `complete_window`,
`scanner_drop_observed`, `stale_event_observed`, `rpc_rate_limited`,
`approximate_trader_identity`, and `partial_protocol_coverage`. A complete
window means the local monotonic timer reached 60 seconds. Fee-payer counts are
an approximate signer proxy and are marked accordingly. A research consumer
should filter these flags before treating records as comparable samples.

Example sanitized record:

```json
{"schema_version":15,"protocol":"raydium_launchlab","launch_account":"Launch...","base_mint":"Base...","quote_mint":"So111...","tx_counts_10_30_60s":[4,11,19],"swap_counts_10_30_60s":[2,8,14],"momentum_score":31.2,"complete_window":true,"quality_flags":{"scanner_drop_observed":false,"stale_event_observed":false,"rpc_rate_limited":false,"approximate_trader_identity":true,"partial_protocol_coverage":false}}
```

### V15 runtime UX and configuration

Startup reports version, cluster, enabled protocols, redacted endpoint hosts,
fetch concurrency, request spacing, stale thresholds, and verbose mode. Set
`VERBOSE=true` for per-transaction decoder lines; normal mode prints important
launches, periodic metrics, and a shutdown summary. Shutdown reports runtime,
notifications, drops, duplicates, stale events, fetch attempts, retries, rate
limits, failures, decoded transactions, launches, snapshots, queue peak, and
lag histogram p50/p95/max. Endpoints are reduced to scheme and host before
printing. `VERBOSE` is the only V15 display setting; existing environment
variables control cluster, endpoints, queue, concurrency, pacing, and freshness.

The backpressure model remains bounded: two listeners share a 2,000-entry input
queue, the fetcher has at most the configured number of jobs, and its output
queue holds 128 fetched transactions. A shared request gate and cooldown keep
retries within the provider's allowance. Stale work is discarded before fetch
or before momentum according to the V13 thresholds. JSONL is append-only and
ignored by Git; rotate it outside the application for long runs.

### V15 validation and benchmark

From the repository root:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build
git diff --check
```

The final benchmark command is read-only and uses the public Mainnet RPC with a
bounded 65-second runtime:

```bash
SOLANA_CLUSTER=mainnet SOLANA_HTTP_URL=https://api.mainnet-beta.solana.com SOLANA_WS_URL=wss://api.mainnet-beta.solana.com/ NO_DNA=1 timeout --preserve-status --signal=INT --kill-after=10s 65s cargo run --quiet
```

The benchmark result and exact counters are recorded in the V15 delivery
report. No new launch is required for a successful run: deterministic fixtures
cover decoder and feature behavior, while live verification confirms listener,
RPC, persistence, and shutdown behavior. Public RPC limits, notification
backlog, no historical backfill, confirmed-fork changes, and incomplete
protocol coverage remain limitations. LaunchLab devnet is disabled because its
program identity was not independently verified from an official Raydium
source. The project does not claim full Solana coverage, production readiness,
or profitability.

### Roadmap

Future work may add separately verified protocols, durable replay/fork handling,
provider-aware ingestion policies, dataset validation tooling, and historical
backfills. Paper trading, backtesting, and financial claims are outside V15.

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

## V14 supported protocols and normalized events

V14 retains the verified Raydium CPMM and CLMM decoders and adds Raydium
LaunchLab on Mainnet. The LaunchLab listener uses program ID
`LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj`. V14 writes normalized records to
`launch_events_v14.jsonl`, launch and pool records to `new_launches_v14.jsonl`,
unknown records to `unknown_instructions_v14.jsonl`, and the existing momentum
and runtime streams with the V14 suffix. These runtime files remain ignored by
Git.

Each normalized `LaunchEvent` contains `protocol`, `event_type`, signature, slot,
block time, local detection time, processing lag, launch or pool account, base
and quote mints when the verified instruction account order supplies them,
creator when the instruction supplies it reliably, source program, instruction
name, and decode status. Reliable event types currently emitted are
`launch_created` for LaunchLab initialization, `pool_created` for CPMM/CLMM pool
creation, and `swap` for the supported CPMM, CLMM, and LaunchLab swap
instructions. The tracker keys state by the normalized launch or pool account
and keeps the 10/30/60-second local observation windows for every supported
protocol.

LaunchLab details were verified against Raydium's official
[raydium-sdk-V2 launchpad instrument source](https://github.com/raydium-io/raydium-sdk-V2/blob/master/src/raydium/launchpad/instrument.ts),
including the discriminators and account order for `initializeV2`,
`initializeWithToken2022`, `buyExactIn`, `buyExactOut`, `sellExactIn`, and
`sellExactOut`. The official [launchpad layout source](https://github.com/raydium-io/raydium-sdk-V2/blob/master/src/raydium/launchpad/layout.ts)
was used to confirm the launch pool fields and mint ordering. Raydium's
[LaunchLab examples](https://github.com/raydium-io/raydium-sdk-V2-demo/tree/master/src/launchpad)
confirm the production program selection. Existing CPMM/CLMM mappings remain
anchored to Raydium's official program repositories linked above.

The scanner does not claim full Solana launch coverage. It does not decode
Raydium AMM v4, Orca, Meteora, Pump.fun, PumpSwap, or other protocols. LaunchLab
devnet support is disabled because this repository has not verified a devnet
program ID from an official Raydium source. LaunchLab migration, vesting,
platform configuration, liquidity management, and event-log payload decoding
remain unsupported. Trade mint pairs and creators are left null when they are
not present in the verified instruction account order; the scanner does not
infer them from arbitrary transaction accounts. V14 still observes confirmed
notifications without historical backfill, fork reconciliation, or unlimited
reordering, and V13 freshness limits can discard delayed work.

## Phase 2 — Historical Research (P1)

The `research` crate is an offline companion to ingestion. It reads
`features_v15.jsonl` and optionally joins a CSV of timestamped prices to create
`research/data/labeled_launches.jsonl`. The live scanner intentionally omits
price, so P1 never invents an outcome: a launch without sufficient future price
observations remains unlabeled and is reported in coverage counts.

The supported price input is deliberately explicit:

```text
protocol,launch_account,base_mint,quote_mint,timestamp_unix,slot,price_quote_per_base,source,source_quality
raydium_cpmm,POOL...,BASE...,QUOTE...,1700000000,123,0.00000123,onchain_reserves_or_verified_export,derived_from_swaps
```

The preferred source is an on-chain derived reserve or swap observation for the
same pool and quote mint, exported by a separate verifier. Solana's official
[`getTransaction` RPC](https://solana.com/docs/rpc/http/gettransaction) provides
confirmed transaction metadata and block time; Raydium's official
[SDK/API](https://github.com/raydium-io/raydium-sdk-V2) documents pool and mint
lookups. The research tool records the supplied `source` and does not call a
provider or use credentials. External providers can be converted to this CSV
through a separately audited adapter.

Outcome definitions use the first price at or after creation as the baseline,
then use the latest observation at or before each horizon, only when the
series also extends to that horizon. Returns are
`price_horizon / baseline - 1`. Maximum return is the greatest pointwise return
inside the window. Maximum drawdown is the lowest `return - running_peak` in the
window. Horizons are 1m, 5m, 15m, and 1h; maximum return and drawdown are
reported for 5m and 15m. This conservative timestamp alignment avoids using a
price observed after a target horizon. Features are read as recorded and are
never recomputed from future prices.

Run the offline evaluator with no network access:

```bash
cargo run -p research -- --features ingestion/data/features_v15.jsonl
cargo run -p research -- --features path/features_v15.jsonl --prices path/prices.csv --output research/data/labeled_launches.jsonl
```

The labeled schema contains protocol, launch account, mints, creation time, the
original V15 feature object, momentum score, quality flags, outcome horizons,
availability, and price-data source. The report gives total/usable/unlabeled
coverage, quality rejections, score buckets (0–20, 20–40, 40–60, 60–80,
80–100), gross win rate, mean/median return, drawdown, expectancy, and simple
feature correlations when there are at least two usable observations. Strategy
helpers are intentionally small and deterministic: score ≥40, score ≥40 with
at least 3 approximate payers, and positive final-window acceleration with
clean quality flags. No parameter sweep or random split is used; a future
chronological train/test split should be added only after enough timestamped
launches exist.

Metrics are gross and unadjusted: fees, slippage, latency, and execution are not
modeled. The current repository contains no labeled historical samples, so it
cannot support a predictive or profitability conclusion. The largest remaining
limitation is obtaining a sufficiently complete, independently verified,
timestamp-aligned on-chain price history across all supported pool types.

## Phase 2 / P2 — Historical Data Acquisition

P2 adds normalized `PriceObservation` records and an idempotent backfill path.
The input price series is an explicit cache or export rather than a guessed
quote. Each row carries protocol, pool/launch account, mints, timestamp, slot,
price in quote per base, source, source quality, and whether it was observed or
derived from swaps. Duplicate account/timestamp observations are collapsed
deterministically, and normalized observations are persisted to
`research/data/price_observations.jsonl`.

The preferred acquisition source remains verified on-chain reserve or swap
reconstruction. The current V15 records do not include raw token amounts or
reserve state, and Raydium's public API documents pool metadata/current data
rather than a complete historical per-pool series. P2 therefore accepts a
provider-neutral normalized CSV produced by a separately verified exporter. No
credential, provider-specific schema, interpolation, or fabricated price is
hidden in the research crate. Rows without enough future observations remain
`missing` or `partial` with an explicit reason.

`backfill` requires an externally acquired CSV with normalized columns:
`protocol,launch_account,base_mint,quote_mint,timestamp_unix,slot,price_quote_per_base,source,source_quality`.
It loads the existing observation cache when present, merges it with the CSV,
sorts and deduplicates by launch account plus timestamp, then rewrites
`research/data/price_observations.jsonl` and
`research/data/labeled_launches.jsonl`. Reimporting the same CSV is idempotent:
it does not duplicate observations, labels, or ordering. The tool never
fabricates prices.

`validate-prices` provides a preflight check, and `backfill` enforces the same
offline gate. It accepts only the normalized nine-column form; it requires a known
launch account, matching protocol and mints, an observation at or after the
launch timestamp, a finite positive price, and non-empty `source` plus
`source_quality`. The supplied provenance fields are preserved in each
`PriceObservation` and therefore in its label. Unknown or unverifiable rows
are rejected rather than silently imported.

```bash
cargo run -p research -- backfill \
  --features ingestion/data/features_v15.jsonl \
  --prices path/prices.csv

cargo run -p research -- validate-prices \
  --features ingestion/data/features_v15.jsonl \
  --prices path/verified_price_export.csv

cargo run -p research -- backfill \
  --features path/features_v15.jsonl \
  --prices path/prices.csv \
  --observations research/data/price_observations.jsonl \
  --output research/data/labeled_launches.jsonl
```

Labels use the first observation at or after creation as the reference price,
then the latest observation at or before each target horizon, provided the
series has reached that horizon. Returns are
`P_horizon / P_reference - 1`; maximum return is the highest pointwise return,
and maximum drawdown is the lowest `return - running peak` in the window. The
tool reports `complete`, `partial`, or `missing`, while V15 scanner quality flags
remain separate from price quality. There is no live collection mode yet:
offline reproducibility and source auditability take priority.

### Offline transaction derivation and paper simulation

`derive-prices` is an offline, fail-closed adapter for locally exported Solana
transaction JSONL. It recognizes only the verified Mainnet LaunchLab program
and its four known swap discriminators, requires account index 4 to match a V15
LaunchLab target, rejects failed transactions, and derives price only from an
unambiguous pair of pre/post token-balance deltas. For each mint exactly two
token-account balances must change by equal and opposite raw amounts with known
decimals. The resulting price is `(quote raw / 10^quote_decimals) / (base raw /
10^base_decimals)`. Wrong programs, missing decimals, zero amounts, unknown
targets, multiple swaps, and extra relevant balance movements are rejected.
Derived observations preserve an optional `source_tx_signature`; older CSV and
JSONL observations remain readable without it. Inventory reports per-horizon
label coverage and backtest-eligible labels in addition to feature readiness.

```bash
cargo run -p research -- derive-prices \
  --features ingestion/data/features_v15.jsonl \
  --transactions path/transactions.jsonl \
  --output research/data/derived_price_observations.jsonl

# backfill accepts validated normalized CSV or derived PriceObservation JSONL
cargo run -p research -- backfill \
  --features ingestion/data/features_v15.jsonl \
  --prices research/data/derived_price_observations.jsonl
```

Historical RPC acquisition is intentionally not automated: this repository has
no verified provider contract for historical signatures and transactions. Export
transactions through a separately audited, read-only process, then derive and
validate locally. No price is fabricated when that export is unavailable.

`rpc_rate_limited` remains a conservative process-wide scanner flag: it means a
rate limit occurred during the process, not that a particular launch is proven
incomplete. The research quality gate therefore continues to reject it until
per-launch fetch-completeness evidence exists.

`paper-trade` is a deterministic simulation only: it has no wallet, signer,
RPC, order submission, leverage, or borrowing code. It accepts clean labeled
samples that meet a fixed score threshold, uses one of the already-labeled
1m/5m/15m/1h horizons, applies configured entry/exit fee and slippage, writes
JSONL trades plus metrics, and rewrites those outputs deterministically.

```bash
cargo run -p research -- paper-trade \
  --features research/data/labeled_launches.jsonl \
  --score-threshold 60 --holding-seconds 300 \
  --starting-capital 10000 --position-size 100 \
  --fee-per-side 0.003 --slippage-per-side 0.005
```

The CLI prints `PAPER SIMULATION ONLY — NO LIVE TRADES`. Tiny samples are
exploratory only; the backtest stays chronological and uses no random shuffle.

## Phase 2 / P3 — Real Dataset Collection and Evaluation

P3 makes collection progress visible without changing the V15 scanner. Run the
read-only ingestion process for an extended period with a provider endpoint
whose rate budget matches the expected traffic. The public Solana RPC is useful
for smoke tests but should not be assumed to provide complete coverage:

```bash
cd ingestion
SOLANA_CLUSTER=mainnet \
SOLANA_HTTP_URL=https://your-configured-rpc.example \
SOLANA_WS_URL=wss://your-configured-rpc.example/ \
FETCH_CONCURRENCY=8 RPC_MIN_REQUEST_INTERVAL_MS=250 \
cargo run --release
```

The scanner appends V15 JSONL records under `ingestion/data/`, keeps runtime
files Git-ignored, bounds queues and fetch concurrency, and flushes output on
ordinary interruption. Existing runtime files are preserved across restarts.
The research reader deduplicates launch/pool accounts before evaluation, so
re-running collection or backfill does not double-count a launch. No wallet or
signer is needed. Better RPC infrastructure can improve coverage, freshness,
and the number of complete windows; it cannot be assumed from this workflow.

Inspect progress from the repository root:

```bash
cargo run -p research -- inventory --features ingestion/data/features_v15.jsonl
cargo run -p research -- backfill --features ingestion/data/features_v15.jsonl --prices path/prices.csv
cargo run -p research -- backtest --features ingestion/data/features_v15.jsonl --prices path/prices.csv
```

`inventory` reports total and unique feature records, protocol counts, complete
and partial windows, scanner quality flags, usable mint coverage, labeling
readiness, and labeled/partial/unlabeled counts. `backtest` writes
`research/data/backtest_report.json`, uses fixed score thresholds of 40, 60,
and 80, separates clean complete and partial samples from scanner-quality
rejections, and reports bucket returns, win rate, mean/median return,
expectancy, drawdown, and correlations. Cost inputs are optional:
`--fee-per-side 0.003 --slippage-per-side 0.005`; defaults are zero. Net
returns apply those percentages on both entry and exit and are clearly marked
as simplified research assumptions.

Interpretation is sample-size dependent: fewer than 30 usable samples are
exploratory only; 30–100 remain weak evidence; 100 or more support preliminary
evaluation, not proof. Records are ordered by creation time for future
chronological threshold exploration and holdout evaluation; random shuffling is
not used. Current coverage is too small for a holdout or predictive claim.
Backtest outputs are gross or simplified net results, never profitability
claims, and do not model realistic execution, MEV, latency, or failed fills.
