# Architecture

```text
Solana WebSocket -> newest-first bounded queue -> paced RPC transaction fetch
-> verified Raydium decode -> LaunchEvent -> 10/30/60 tracker -> V15 features
-> offline transaction export -> strict price derivation -> validation/cache
-> labels -> chronological backtest -> paper-only simulation
```

The ingestion process owns live observations, bounded freshness, retries,
deduplication, and runtime metrics. Research owns immutable feature reading,
historical evidence validation, labels, and evaluation. Paper simulation owns no
network or signing capability and persists only deterministic simulated trades.

Historical extraction fails closed. A LaunchLab price requires a successful
exported transaction, a verified program/discriminator/account layout, known
target mints/decimals, and unambiguous instruction-scoped token transfers (with
an owner-scoped balance fallback). Exact-in input and principal output amounts
must agree; fees, unrelated movements, and ambiguous routing are rejected. This
protects the research path from guessed prices and unrelated token movements.
Derived observations retain an optional source transaction signature for audit
traceability to the local export.

Quality flags preserve scanner evidence. Process-wide rate limiting is not
reinterpreted as per-launch completeness without additional evidence.
