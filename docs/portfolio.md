# Portfolio material

## Project pitch

Solana Trenches is a Rust/Solana data-infrastructure project that turns noisy, rate-limited Solana program notifications into bounded, freshness-aware, auditable launch observations. It separates live best-effort ingestion from offline historical research, verifies protocol semantics before deriving data, and carries uncertainty into labels and evaluation rather than overstating coverage or financial conclusions.

## Resume bullets

- Built a Rust/Tokio Solana ingestion pipeline using targeted WebSocket subscriptions and paced `getTransaction` RPC retrieval.
- Designed bounded fresh-first queues, signature deduplication, monotonic freshness limits, and backpressure metrics to make overload and coverage gaps observable.
- Implemented verified Raydium CPMM/CLMM/LaunchLab instruction decoding across outer and inner instructions with instruction-scoped pool attribution.
- Produced deterministic 10/30/60-second launch features with fixed memory bounds, explainable activity scores, and explicit data-quality flags.
- Developed a fail-closed offline LaunchLab price-derivation workflow that validates transfers/balance deltas and preserves source transaction signatures.
- Added horizon-aware labels, chronological backtesting, and deterministic costed paper simulation with no signing, wallet, or order-submission capability.

## 60-second interview explanation

“The difficult part was not subscribing to Solana logs; it was treating them honestly. Logs give signatures quickly, but transaction RPC is rate-limited and can arrive late or out of order. I built a bounded pipeline where work has an age, fetches share pacing and cooldown, duplicates are controlled, and the tracker has one owner. The decoder only calls a swap when the program, discriminator, and account layout match. For research, I kept live ingestion separate from a local, fail-closed historical path that records source signatures. The result is a small but reproducible Rust indexer-style system that demonstrates reliability tradeoffs without claiming complete coverage or trading performance.”

## Difficult decisions

1. **Bounded queues vs. full capture:** bounded queues avoid memory failure and stale processing; metrics make loss visible.
2. **Shared pacing vs. per-worker throughput:** a shared gate prevents concurrency from exceeding provider allowance, even if it limits peak throughput.
3. **Monotonic windows vs. chain-time reconstruction:** stable local timing is safer for live observation, but does not claim historical completeness.
4. **Strict parsing vs. broad heuristic coverage:** verified layouts reject more data but prevent misattributed swaps and prices.
5. **Quality gates vs. larger reported samples:** retaining warnings through backtests prevents historical price availability from being mistaken for clean scanner evidence.

## Evidence and limitations

The repository contains offline unit tests for queueing, retries, parsing, feature windows, quality gates, derivation, labels, backtesting, and paper simulation, plus a fully offline synthetic end-to-end demo. The checked-in historical pilot documents 10 targets and 35 real observations, with two 1-minute, one 5-minute, one 15-minute, and zero 1-hour labels; it has zero scanner-quality-accepted backtest rows. The current data is therefore unsuitable for performance or alpha claims.

The scanner is best-effort confirmed-event infrastructure: it has no replay, fork reconciliation, unlimited reorder buffer, or broad protocol coverage. Paper simulation is historical and deterministic, not live forward testing.

## Role keywords

Junior Rust Developer · Solana Backend Developer · Blockchain Data/Indexer Engineer · Web3 Infrastructure Engineer · Tokio · async Rust · WebSocket ingestion · RPC resilience · data provenance · on-chain decoding · backpressure · deterministic research tooling
