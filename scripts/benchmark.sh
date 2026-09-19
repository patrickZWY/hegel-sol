#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="$repo_root/target/release/hegel-sol"
stateless_cases="${HEGEL_SOL_BENCH_STATELESS_CASES:-2000}"
stateful_cases="${HEGEL_SOL_BENCH_STATEFUL_CASES:-200}"
stateful_steps="${HEGEL_SOL_BENCH_STATEFUL_STEPS:-100}"

cargo build --release --locked -p hegel-sol --manifest-path "$repo_root/Cargo.toml"

printf '\nStateless collections: %s cases, array 64, bytes 256\n' "$stateless_cases"
"$binary" test \
  --root "$repo_root/contracts" \
  --match-contract BenchmarkCollectionsTest \
  --test-cases "$stateless_cases" \
  --seed 17 \
  --database off \
  --max-array-len 64 \
  --max-byte-len 256 \
  --show-timings

printf '\nStateful depth: %s cases x %s steps\n' "$stateful_cases" "$stateful_steps"
"$binary" test \
  --root "$repo_root/contracts" \
  --match-contract BenchmarkStatefulTest \
  --test-cases "$stateful_cases" \
  --step-count "$stateful_steps" \
  --seed 17 \
  --database off \
  --show-timings
