#!/usr/bin/env bash
# Deterministic, synthetic, no-network research demonstration.
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "$0")/.." && pwd)
feature_file="$repo_root/research/fixtures/demo_features_v15.jsonl"
transaction_file="$repo_root/research/fixtures/demo_launchlab_transactions.jsonl"
work_root=$(mktemp -d "${TMPDIR:-/tmp}/solana-trenches-demo.XXXXXX")

cleanup() {
  if [[ "${DEMO_KEEP_OUTPUT:-0}" == "1" ]]; then
    printf 'Synthetic demo output retained at %s\n' "$work_root"
  else
    rm -rf -- "$work_root"
  fi
}
trap cleanup EXIT

run_case() {
  local destination=$1
  mkdir -p "$destination"
  CARGO_NET_OFFLINE=true cargo run -q -p research -- derive-prices --features "$feature_file" --transactions "$transaction_file" --output "$destination/derived.jsonl"
  CARGO_NET_OFFLINE=true cargo run -q -p research -- validate-prices --features "$feature_file" --prices "$destination/derived.jsonl"
  CARGO_NET_OFFLINE=true cargo run -q -p research -- backfill --features "$feature_file" --prices "$destination/derived.jsonl" --observations "$destination/observations.jsonl" --output "$destination/labeled.jsonl"
  CARGO_NET_OFFLINE=true cargo run -q -p research -- backtest --features "$feature_file" --prices "$destination/derived.jsonl" --report "$destination/backtest.json" --fee-per-side 0.001 --slippage-per-side 0.002
  CARGO_NET_OFFLINE=true cargo run -q -p research -- paper-trade --features "$destination/labeled.jsonl" --output "$destination/trades.jsonl" --metrics "$destination/metrics.jsonl" --score-threshold 60 --holding-seconds 300 --starting-capital 1000 --position-size 100 --fee-per-side 0.001 --slippage-per-side 0.002
}

cd "$repo_root"
printf 'SYNTHETIC OFFLINE DEMO — no RPC, wallets, signing, or submissions\n'
run_case "$work_root/first"
run_case "$work_root/second"
for output in derived.jsonl observations.jsonl labeled.jsonl trades.jsonl metrics.jsonl backtest.json; do
  cmp -s "$work_root/first/$output" "$work_root/second/$output"
done
printf 'deterministic=true derived_observations=%s labels=%s simulated_trades=%s\n' \
  "$(wc -l < "$work_root/first/derived.jsonl")" \
  "$(wc -l < "$work_root/first/labeled.jsonl")" \
  "$(wc -l < "$work_root/first/trades.jsonl")"
