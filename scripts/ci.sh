#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
forge fmt --root contracts --check
forge build --root contracts
cargo test --workspace --locked
