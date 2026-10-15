# Repository guidelines

Conventions for contributors and coding agents. `README.md` is the user's entry point;
this file is the maintainer's.

## Workflows

Use `cargo x` for every build, test, lint, and formatting command, and read
`cargo x --help` and the subcommand's `--help` before running one.

```sh
cargo x lint            # rustfmt, clippy, forge fmt, typos; --fix applies fixes
cargo x build --locked  # every crate and every Foundry project
cargo x test            # unit, integration, and pinned real-contract tests
cargo x demo            # three intentional failures, found and shrunk
cargo x bench           # release-mode scaling workloads
cargo x validate        # pinned real-contract properties under Forge and hegel-sol
cargo x profile         # wall time, CPU time, and peak memory for one real workload
```

CI runs the first three on every push and pull request, and the committed pre-push hook
runs the same; enable it with `git config core.hooksPath .github/hooks`. Foundry projects
are discovered by their `foundry.toml`, so a new one needs no registration.

Two versions are pinned by hand. The Rust MSRV is `rust-version` in `Cargo.toml` and the
first matrix entry in `.github/workflows/ci.yml`; the Foundry release is `version` in
`.github/actions/setup/action.yml`. Bump each in both places at once.

## Layout

Every top-level directory serves one audience, and that decides where a new file goes.

- `crates/hegel-sol` is the runner. Keep the layering described in `docs/design.md`: the
  engine FFI under the EVM host, under the Foundry and solc adapters, under the runner.
- `solidity/` holds `Hegel.sol`, the file test projects import. Bump
  `HEGEL_PROTOCOL_VERSION` there and `PROTOCOL_VERSION` in `protocol.rs` together
  whenever a selector or the meaning behind one changes.
- `examples/` holds contracts a reader is meant to study. Every test in it runs end to
  end, and the integration tests pin the counterexamples the README promises.
- `crates/tests-integration/projects/behaviour` holds one contract per runner behaviour;
  each runs to completion. `projects/broken` holds projects the runner must refuse.
- `crates/xtask` holds the workflows, and `crates/xtask/benchmarks` the workloads for
  `cargo x bench`, which are never tests. `docs/` holds the design document and
  reference material that is a contract with users, such as the JSON report schema.

## Tests

Name an integration test for the behaviour it pins, not for the phase or feature that
introduced it. Assert at the behaviour boundary: read a reported source line back from
the file instead of pinning a line number, use discovery to test scheduling, and never
assert only that a workload passed. Give every fixture contract a NatSpec comment naming
the behaviour it exists for.

## Style

Rust is formatted by rustfmt and linted by Clippy with warnings denied; Solidity is
formatted by `forge fmt`. Comments and doc comments explain why, not what. Error
messages name the file or option the reader should change.

## Documentation

- `README.md`: what the runner does and how to use it. Keep it an entry point.
- `docs/design.md`: architecture, the engine and revm API facts the code relies on, and a
  dated decision log. Append an entry when a decision is made or reversed rather than
  rewriting history.
- `docs/reporting.md`: the JSON contract. A change to it follows the versioning rules it
  states.

Wrap Markdown prose at 90 columns. Link to a file the first time it is named.

## Changelog

Update `CHANGELOG.md` for user-visible changes by comparing the final behaviour with the
latest release tag, not by recording the commits in the current cycle. Before adding a
bug-fix entry, confirm the faulty behaviour was released; if it was not, describe only
the final contract in the relevant feature entry. Include CLI and `Hegel.sol` changes,
report-schema changes, compatibility changes, and meaningful performance improvements.
Exclude tests, refactors, documentation, CI, and tooling unless they change observable
behaviour. Write each entry from the user's perspective, with migration guidance for a
breaking change.

## Releasing

Turn the changelog's pending section into `## X.Y.Z - date`, set `version` in
`Cargo.toml`, and commit. Once CI on `main` is green, push a `vX.Y.Z` tag. The release
workflow reruns the checks, builds a Linux binary, and publishes it with a checksum as a
GitHub release.

## Commits and pull requests

Prefix titles with the kind of change (`feat`, `fix`, `docs`, `refactor`, `test`, `ci`,
`chore`) and keep descriptions short. Say what changed for the user first and why second.
