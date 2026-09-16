#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

runner="$repo_root/target/debug/hegel-sol"

section() {
    printf '\n\033[1;36m%s\033[0m\n\n' "$1"
}

run_expected_failure() {
    local expected_test="$1"
    shift

    local output
    local status
    set +e
    output="$($runner "$@" 2>&1)"
    status=$?
    set -e
    printf '%s\n' "$output"

    if [[ $status -ne 1 || $output != *"FAIL $expected_test"* ]]; then
        printf '\nDemo step failed unexpectedly (exit %s, expected test %s).\n' \
            "$status" "$expected_test" >&2
        exit 1
    fi
}

section "Building hegel-sol"
cargo build --quiet --locked -p hegel-sol

section "1/3 Stateless property: find and shrink an ERC20 accounting bug"
run_expected_failure "ERC20SpikeTest.test_transfer_preserves_supply" \
    test --root contracts --match-contract ERC20SpikeTest \
    --seed 1 --test-cases 50 --database off --verbosity quiet

section "2/3 Stateful property: shrink a bug to the shortest rule sequence"
run_expected_failure "StatefulCounterTest.stateful" \
    test --root contracts --match-contract StatefulCounterTest \
    --seed 1 --test-cases 100 --database off --verbosity quiet --show-statistics

section "3/3 Solidity diagnostics: decode a custom error and console.log"
run_expected_failure "CustomErrorTest.test_custom_error" \
    test --root contracts --match-contract CustomErrorTest \
    --seed 1 --test-cases 1 --database off --verbosity quiet

section "Demo complete"
printf 'All three expected failures were found, minimized, and reported.\n'
