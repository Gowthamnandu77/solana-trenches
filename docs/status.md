# Status

## Complete

- V15 Raydium ingestion, bounded freshness, decoding, feature persistence, and tests.
- Offline price export validation, cache, horizon-aware labels, chronological backtest.
- Offline LaunchLab transaction-export derivation with fail-closed validation
  and optional transaction-signature provenance.
- Paper-only deterministic simulation with costs and JSONL persistence.

## Real-data complete

- Archive-RPC transaction acquisition and LaunchLab instruction-scoped price
  derivation are verified for all ten targets.
- 35 unique real observations are available: two 1m labels, one 5m label, and
  one 15m label. No real 1h label exists.

## Blocked by external data

- A complete +1h market-activity series for at least one target.
- Scanner-quality acceptance remains separate from historical price coverage;
  all current V15 rows retain conservative rate-limit/identity warnings.

## Environment prerequisite

- The Anchor integration test needs `target/deploy/solana_trenches.so` from an
  SBF build. The current build is blocked by a program-ID/keypair mismatch and
  unavailable crate downloads; neither is changed automatically.

## Future work

- Provider-specific read-only export adapter after retention, rate limits, and response semantics are verified.
- Per-launch evidence for RPC-induced incompleteness.
- Larger chronological holdout evaluation after adequate clean labeled coverage.
