# Status

## Complete

- V15 Raydium ingestion, bounded freshness, decoding, feature persistence, and tests.
- Offline price export validation, cache, horizon-aware labels, chronological backtest.
- Offline LaunchLab transaction-export derivation with fail-closed validation.
- Paper-only deterministic simulation with costs and JSONL persistence.

## Blocked by external data

- A verified historical RPC/provider contract or audited transaction export for the real ten launches.
- Real historical observations, labels, and any meaningful performance conclusion.

## Future work

- Provider-specific read-only export adapter after retention, rate limits, and response semantics are verified.
- Per-launch evidence for RPC-induced incompleteness.
- Larger chronological holdout evaluation after adequate clean labeled coverage.
