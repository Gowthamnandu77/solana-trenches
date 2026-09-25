# Project status

## Implemented and tested offline

- V15 Raydium CPMM, CLMM, and supported Mainnet LaunchLab ingestion with bounded freshness, deduplication, pacing/retries, decoding, momentum windows, persistence, and runtime metrics.
- Offline normalized price validation/cache, horizon-aware labels, chronological backtesting, and deterministic costed paper simulation.
- Fail-closed local LaunchLab transaction-export price derivation with optional source transaction signature provenance.

## Documented historical pilot (not live telemetry)

The checked-in project documentation supports these historical read-only pilot facts:

- 10 LaunchLab targets were processed.
- 35 unique real price observations were retained.
- Labels available: two at 1 minute, one at 5 minutes, one at 15 minutes, and none at 1 hour.
- Zero rows were accepted by the scanner-quality backtest gate.

The repository docs currently do **not** record the 1,661-transaction or 67-verified-swap-candidate totals. They should not be presented as validated repository metrics until their source evidence is committed or documented. None of these pilot counts are current live metrics, trading performance, or evidence of alpha.

## Why the current data cannot support a result claim

No complete one-hour series exists for the pilot, so there is no 1-hour label. More importantly, historical price coverage and scanner quality are independent: current V15 rows retain conservative warnings, so they fail the quality gate even where a short-horizon price label exists. The backtest therefore has zero accepted samples. Historical paper simulation is a replay over labels; it is not live forward paper trading.

## External limitations and next evidence needed

- A larger chronological sample with clean, complete scanner-quality records and sufficient future prices.
- Per-launch evidence to distinguish process-wide rate limiting from launch-specific incompleteness.
- A provider-specific, read-only acquisition adapter only after retention, rate limit, and response semantics are independently verified.
- Broader protocol coverage only after verified account-layout and discriminator support is added.

## Anchor workspace-test prerequisite

`cargo test --workspace` includes an Anchor/LiteSVM integration test that requires `target/deploy/solana_trenches.so` from an SBF build. The existing project notes a program-ID/keypair mismatch and unavailable crate downloads as blockers; this documentation pass does not change those prerequisites or attempt to work around them. Ingestion and research tests do not require SBF artifacts or network access.
