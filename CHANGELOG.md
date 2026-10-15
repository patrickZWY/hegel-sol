# Changelog

User-visible changes, described against the latest release rather than commit by
commit.

## 0.2.0 - 2026-10-15

### Features

- Forge-style invariant tests run unchanged. A test with `invariant*` functions that
  registers handler contracts through `targetContracts()`, or leaves every contract its
  `setUp()` deployed as a target, has those contracts' state-changing functions driven
  by the state machine, minus any `excludeContracts()` names. A reverting target call
  is a skipped step, as under Forge. The default invariant prefix is now Forge's bare
  `invariant`, which still matches `invariant_`.
- forge-std's assertions work: the `vm.assert*` cheatcode family reverts with a
  readable message such as `assertion failed: 0 != 7`, and classic DSTest suites that
  record a failure through `vm.store` and `failed()` are reported as failed with their
  logged diagnostics. `vm.store` and `vm.load` are emulated for any account.
- Solady's own ERC20 test suite runs unchanged and passes every test Forge passes.
  The emulated cheatcodes grew by `expectEmit` in all four overloads, `addr`, and
  `sign`, and the EVM code-size limit no longer applies to test contracts, as in
  Forge. `cargo x validate` now runs that suite under both runners.
- A test that is overloaded, as Forge suites do to pair a fixed case with a fuzzed
  one, is reported and selected by its full signature, such as
  `ERC20Test.testMint(address,uint256)`, so the two stay distinct in reports, in the
  example database, and in a replay.
- `IHegelVm` in `Hegel.sol` declares the `bytes4` overload of `expectRevert` and the
  new cheatcodes. Projects that only use the earlier subset need no update.
- A test contract that fails to deploy is reported by name with the revert reason or
  halt.

### Fixes

- A call that `expectRevert` matched returned no data, so a test that read the call's
  declared return value reverted while decoding it and was reported as a failure Forge
  would not report. The runner now returns zero-filled data, as Forge does.

## 0.1.0 - 2026-10-15

First release.

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
  sender of each stateful call, and a project-relative Solidity source position. A
  stateful counterexample prints as `vm.prank` and handler calls with their arguments,
  and the printed reproduce command selects the failing test and repeats the options
  that shape its draws, so it pastes back unchanged. A failure that did not reproduce
  on every replay carries the engine's caveat; a `--reproduce` blob the test has
  outgrown is reported as `overrun` rather than as a pass.
- Generation is guided by basic-block coverage; `--no-coverage-target` disables it.
  `--actors`, `--allow-rule-reverts`, `--max-array-len`, `--max-byte-len`, and the
  `--test-prefix`, `--rule-prefix`, and `--invariant-prefix` options tune a run.
- `--root` defaults to the current directory. `Hegel.sol` is published from
  `solidity/src/` and can be remapped from a Foundry `lib/` checkout.
- `cargo x validate` checks pinned OpenZeppelin, Solady, and Uniswap v2 properties
  under Forge and hegel-sol; `cargo x test` includes this campaign. `cargo x profile`
  measures a selected real-contract workload's time and peak memory.
