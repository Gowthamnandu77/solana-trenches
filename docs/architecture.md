# Architecture

## System boundary

```text
logsSubscribe (CPMM / CLMM / LaunchLab)
  -> bounded freshest-first notification queue
  -> bounded, paced getTransaction jobs
  -> verified outer + inner instruction decoder
  -> single-owner launch tracker and V15 JSONL features
  -> local, read-only historical transaction export
  -> fail-closed price observations with source signatures
  -> horizon labels -> chronological backtest / deterministic paper simulation
```

`ingestion` owns live observation state, queueing, RPC behavior, decoding, timers, metrics, and append-only JSONL. `research` owns immutable feature reading, historical evidence validation, labels, evaluation, and simulation. The Anchor program is a separate counter example and is not part of this data path.

## WebSocket to RPC ingestion

Two targeted WebSocket listeners receive signatures and send owned notification values to one bounded intake path. The dispatcher deduplicates signatures, rejects work that is already too old, and runs only a configured number of fetch jobs. A shared request-start gate controls all jobs and retries; a bounded fetched-transaction queue gives the coordinator sole ownership of tracker mutation. This avoids a shared mutable pool map and makes overload behavior explicit.

The tracker uses monotonic elapsed time for half-open 10/30/60-second windows. It has bounded active pools, retained signatures, payer sets, and fixed buckets. It writes feature and snapshot JSONL with flags such as `complete_window`, rate limiting, stale events, and partial protocol coverage. “Complete” means the local timer elapsed—not full chain coverage.

## Reliability tradeoffs

| Concern | Mechanism | Intentional limitation |
| --- | --- | --- |
| Traffic bursts | Bounded queues, fresh-first admission, queue-age limits | Notifications can be dropped or discarded as stale; metrics disclose it. |
| Duplicate logs/reconnects | Global, pending, and per-pool signature deduplication | Bounded histories eventually evict old signatures. |
| RPC rate limits | Shared pacing gate, cooldown, retry budget, deadlines | Conservative pacing cannot guarantee Mainnet-wide coverage on a weak endpoint. |
| Out-of-order completion | Creation/transaction processing under freshness rules | No unbounded reorder buffer or historical replay. |
| Protocol ambiguity | Known IDs, discriminators, and account indices | Unsupported instructions are activity at most, never fabricated swaps. |
| Memory growth | Fixed queue, pool, signature, and payer capacities | Counts can cap; JSONL needs external rotation for long runs. |

## Verified decoding and feature quality

Raydium CPMM, CLMM, and supported Mainnet LaunchLab instructions are recognized only after program-ID, discriminator, and relevant account-layout checks. Both outer instructions and inner CPI instructions are considered, which matters for routed swaps. Pool attribution is instruction-scoped; a transaction-level account mention is insufficient.

Features capture local activity over 10/30/60 seconds: transaction and decoded-swap counts, approximate unique fee-payer counts, rates, rate changes, acceleration, and a deterministic bounded activity score. The score does not use prices, volume, direction, PnL, or a claim about economic trader identity. Fee payers are a signer proxy and explicitly approximate.

## Historical evidence, labels, and safeguards

Historical acquisition is explicit, read-only, and local-export based. `fetch-transactions` may contact a configured RPC only when deliberately invoked; it stores raw output locally and Git ignores it. `derive-prices` is offline and fails closed: it requires a successful transaction, verified LaunchLab program/discriminator/layout, target mints/decimals, and unambiguous instruction-scoped transfers or owner-scoped balance deltas. Ambiguous routing, fees, unrelated movements, mismatched exact-in amounts, and unknown targets are rejected. Derived observations retain the source transaction signature.

Labels use a first observation at or after creation as baseline and an observation at or before each requested horizon only if the series reaches that horizon. This prevents future-price leakage. Backtests sort by creation time and use chronological evaluation; paper simulation only reads labeled files, applies fixed costs, and writes deterministic simulated trades. Neither component contains RPC, wallet, signing, borrowing, or order-submission code.

## Why the real pilot has no accepted backtest sample

The documented historical pilot has price coverage for a small subset of horizons, including no complete 1-hour label. A label is not enough for backtesting: the quality gate also requires a complete scanner window with no drop, stale-event, rate-limit, or partial-coverage warning. The current V15 rows retain conservative warnings, so zero rows pass the scanner-quality gate. This separates price availability from ingestion evidence; it is not a trading result.
