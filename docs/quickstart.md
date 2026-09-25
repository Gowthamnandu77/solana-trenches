# Quick start

## Prerequisites

- Rust matching `rust-toolchain.toml` (workspace minimum: Rust 1.89).
- For the Anchor integration test only: Solana CLI, Anchor tooling, and an SBF build that produces `target/deploy/solana_trenches.so`.
- No Solana account, wallet, RPC endpoint, or network access is required for the offline demo or the ingestion/research unit tests.

On Windows Command Prompt, enter WSL first and then navigate using Linux paths:

```cmd
wsl
cd /home/cure/solana-trenches
```

## Offline demo

```bash
cd /home/cure/solana-trenches
./scripts/offline-demo.sh
```

It uses only committed synthetic fixtures, writes to a new temporary directory, and sets Cargo offline mode. The fixture has one clean LaunchLab feature and two manually inspectable synthetic transactions. Expected outputs are two derived/validated/merged observations, one labeled launch, one quality-accepted 5-minute backtest sample, and one simulated trade. These figures are fixture behavior only—not real chain data or a trading claim.

Run the script again. It reruns the pipeline in a second empty temporary directory and verifies that observations, labels, trades, metrics, and the report are byte-for-byte identical. The temporary output directory is removed on exit unless `DEMO_KEEP_OUTPUT=1` is supplied.

## Test and lint commands

```bash
cargo fmt --check
cargo clippy -p ingestion -- -D warnings
cargo test -p ingestion
cargo clippy -p research -- -D warnings
cargo test -p research
cargo clippy --workspace -- -D warnings
cargo run -p research -- --help
```

## Optional read-only scanner

Copy the placeholder template without placing any credential in Git:

```bash
cp ingestion/.env.example ingestion/.env
```

`SOLANA_CLUSTER`, `SOLANA_HTTP_URL`, and `SOLANA_WS_URL` select the cluster and matching endpoints. `FETCH_CONCURRENCY`, `RPC_MIN_REQUEST_INTERVAL_MS`, `INPUT_QUEUE_CAPACITY`, `MAX_FETCH_START_AGE_MS`, and `MAX_MOMENTUM_EVENT_AGE_MS` tune bounded behavior. Endpoint values are intentionally absent from the template.

Start the scanner from repository root with `cargo run -p ingestion`. It is read-only but will connect to its configured/public defaults; do not use it for offline validation. `--help` is not a help-only interface for this binary. Generated JSONL stays under ignored `ingestion/data/`.

## Research CLI

```bash
cargo run -p research -- --help
cargo run -p research -- inventory --features path/to/features_v15.jsonl
cargo run -p research -- validate-prices --features path/to/features_v15.jsonl --prices path/to/prices.csv
cargo run -p research -- derive-prices --features path/to/features_v15.jsonl --transactions path/to/local_export.jsonl --output /tmp/derived.jsonl
cargo run -p research -- backfill --features path/to/features_v15.jsonl --prices /tmp/derived.jsonl --observations /tmp/observations.jsonl --output /tmp/labeled.jsonl
cargo run -p research -- backtest --features path/to/features_v15.jsonl --prices /tmp/derived.jsonl --report /tmp/report.json
cargo run -p research -- paper-trade --features /tmp/labeled.jsonl --output /tmp/trades.jsonl --metrics /tmp/metrics.jsonl
```

`fetch-transactions` is the one research command that may contact an RPC and requires a local ignored `ingestion/.env`; it is not part of the demo. It performs only read-only RPC calls and writes raw exports to an ignored directory.
