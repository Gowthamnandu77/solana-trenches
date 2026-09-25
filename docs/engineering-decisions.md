# Engineering decisions

## 1. Bound queues instead of retaining every notification

Unbounded queues turn provider slowdown into unbounded memory and increasingly stale work. The scanner caps intake and fetched-transaction queues, tracks high-water marks and drops, and prefers fresh work. The tradeoff is explicit best-effort coverage rather than a false completeness claim.

## 2. One shared RPC pacing gate

Fetch concurrency overlaps network latency, but every request start—including retries—passes through one gate and rate-limit cooldown. This avoids accidentally multiplying provider pressure by worker count. It sacrifices peak throughput when an endpoint is weak, which is safer and easier to reason about.

## 3. Monotonic freshness over wall-clock ordering

Local monotonic elapsed time controls queue age and 10/30/60-second windows, avoiding effects from wall-clock adjustment. Late arrivals cannot rewrite emitted windows. The cost is that this is scanner-observation time, not a historical reconstruction of first on-chain seconds.

## 4. Protocol evidence must be instruction-scoped

The decoder verifies program ID, discriminator, and relevant account placement for outer and inner instructions. A transaction that merely mentions Raydium does not become a swap. This deliberately leaves unsupported formats unknown rather than manufacturing semantic data.

## 5. Historical price attribution fails closed

The research path accepts a price only when target, program, swap discriminator, mints, decimals, and principal transfers/balance deltas agree. It keeps a source signature. Ambiguous routes, fees, and unrelated transfers are rejected, trading recall for auditability.

## 6. Data quality gates survive into evaluation

A short-horizon label does not erase scanner warnings. Backtests and simulations require clean complete-window evidence, sort chronologically, and warn on small samples. The result is intentionally less flattering than treating every exported price as a comparable sample.
