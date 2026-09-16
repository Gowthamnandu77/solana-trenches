# Status

## Complete

- V15 Raydium ingestion, bounded freshness, decoding, feature persistence, and tests.
- Offline price export validation, cache, horizon-aware labels, chronological backtest.
- Offline LaunchLab transaction-export derivation with fail-closed validation
  and optional transaction-signature provenance.
- Paper-only deterministic simulation with costs and JSONL persistence.

## Real-data complete

- None. The repository intentionally contains no fabricated historical prices,
  labels, backtest results, or paper-simulation results.

## Blocked by external data

- A verified historical RPC/provider contract or audited transaction export for the real ten launches.
- Real historical observations, labels, and any meaningful performance conclusion.

## Environment prerequisite

- The Anchor integration test needs `target/deploy/solana_trenches.so` from an
  SBF build. The current build is blocked by a program-ID/keypair mismatch and
  unavailable crate downloads; neither is changed automatically.

## Future work

- Provider-specific read-only export adapter after retention, rate limits, and response semantics are verified.
- Per-launch evidence for RPC-induced incompleteness.
- Larger chronological holdout evaluation after adequate clean labeled coverage.
