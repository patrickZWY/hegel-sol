# Repository guidelines

Conventions for contributors and coding agents. `README.md` is the user's entry point;
this file is the maintainer's.

## Workflows

Use `cargo x` for every build, test, lint, and formatting command, and read
`cargo x --help` and the subcommand's `--help` before running one. CI and the pre-push
hook run `cargo x lint`, `cargo x build --locked`, and `cargo x test`, so a change that
passes those locally passes CI. `cargo x lint --fix` applies formatting and lint fixes.
Foundry projects are discovered by their `foundry.toml`, so a new one needs no
registration.

Two versions are pinned by hand. The Rust MSRV is `rust-version` in `Cargo.toml` and the
first matrix entry in `.github/workflows/ci.yml`; the Foundry release is `version` in
`.github/actions/setup/action.yml`. Bump each in both places at once.

## Layout

Every top-level directory serves one audience, and that decides where a new file goes.

- `crates/hegel-sol` is the runner. Keep the layering described in `DESIGN.md`: the
  engine FFI under the EVM host, under the Foundry and solc adapters, under the runner.
- `solidity/` holds `Hegel.sol`, the file test projects import. Bump
  `HEGEL_PROTOCOL_VERSION` there and `PROTOCOL_VERSION` in `protocol.rs` together
  whenever a selector or the meaning behind one changes.
- `examples/` holds contracts a reader is meant to study. Every test in it runs end to
  end, and the integration tests pin the counterexamples the README promises.
- `benchmarks/` holds workloads for `cargo x bench`. They are never tests.
- `crates/tests-integration/projects/behaviour` holds one contract per runner behaviour;
  each runs to completion. `projects/broken` holds projects the runner must refuse.
- `crates/xtask` holds the workflows. `docs/` holds reference material that is a
  contract with users, such as the JSON report schema.

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
- `DESIGN.md`: architecture, the engine and revm API facts the code relies on, and a
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

## Commits and pull requests

Prefix titles with the kind of change (`feat`, `fix`, `docs`, `refactor`, `test`, `ci`,
`chore`) and keep descriptions short. Say what changed for the user first and why second.
