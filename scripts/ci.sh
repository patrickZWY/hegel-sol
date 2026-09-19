#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
for project in contracts fixtures; do
    forge fmt --root "$repo_root/$project" --check
    forge build --root "$repo_root/$project"
done
cargo test --workspace --locked
