# Changelog

User-visible changes, described against the latest release rather than commit by
commit. Nothing has been released yet, so the section below describes the runner as it
will first ship.

## Unreleased

### Features

- `hegel-sol test` runs stateless `test*` properties and stateful `rule_*` /
  `invariant_*` machines in a Foundry project, with shrinking, reproduce blobs, a
  persistent example database, `--jobs`, `--shard`, `--fail-fast`, `--list`, and a
  versioned `--json` report documented in `docs/reporting.md`.
- Ordinary Forge fuzz tests run unchanged. ABI arguments are generated for scalars,
  bytes, strings, arrays, tuples, and structs, and the `prank`, `startPrank`,
  `stopPrank`, `deal`, `warp`, `roll`, `expectRevert`, `label`, and `assume` cheatcodes
  are emulated in-process.
- `Hegel.sol` provides named and auto-named draws, `assume`, spans, notes, labelled
  targets, events, and pools. It carries a protocol version the runner checks before
  running anything, so a stale copy is reported by name.
- Failures report decoded custom errors, captured `console.log` output, notes, the
  sender of each stateful call, and a project-relative Solidity source position.
- Generation is guided by basic-block coverage; `--no-coverage-target` disables it.
  `--actors`, `--allow-rule-reverts`, `--max-array-len`, `--max-byte-len`, and the
  `--test-prefix`, `--rule-prefix`, and `--invariant-prefix` options tune a run.
- `--root` defaults to the current directory. `Hegel.sol` is published from
  `solidity/src/` and can be remapped from a Foundry `lib/` checkout.
- `cargo x validate` checks pinned OpenZeppelin, Solady, and Uniswap v2 properties
  under Forge and hegel-sol; `cargo x test` includes this campaign. `cargo x profile`
  measures a selected real-contract workload's time and peak memory.
