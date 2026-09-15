# Continuous integration and delivery

Every push and pull request targeting `main` runs three CI checks:

- **Rust quality**: `cargo fmt` and Clippy with warnings denied.
- **Solidity quality and build**: `forge fmt` and `forge build`.
- **Tests**: all Rust unit tests and Solidity-backed integration tests.

Run the same checks locally with:

```sh
./scripts/ci.sh
```

This clone uses the committed pre-push hook in `.githooks/pre-push`, so a local push
is rejected when any check fails. Enable it in another clone with:

```sh
git config core.hooksPath .githooks
```

GitHub branch protection should require all three checks and require branches to be
up to date before merging. For a private repository, that feature requires a GitHub
plan which supports protected branches.

## Releases

Pushing a semantic-version tag such as `v0.1.0` runs the full check suite, builds the
release binary, creates a SHA-256 checksum, and publishes both files in a GitHub
release. The release is not created if any quality or test step fails.
