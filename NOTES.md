# Phase 0 spike results

Implemented and measured on 2026-09-15 with Rust 1.100.0-nightly, Forge 1.7.1,
`hegeltest-c` 0.42.3, and revm 43.0.2.

## Result

The end-to-end loop works. With seed 1 and 50 cases, the ERC20 spike finds the
deliberate supply bug and shrinks it to:

```solidity
address from = 0x0000000000000000000000000000000000000000;
address to = 0x0000000000000000000000000000000000000001;
uint256 amount = 3;
```

The search, shrink, final replay, and reporting take about 50 ms after compilation.
The emitted `HEGEL_SOL_REPRODUCE` blob replays the same panic and draw trace.

## Differences from DESIGN.md

- Solidity reserves elementary type names and `event`, so methods named `uint256`,
  `address`, `bool`, `bytes`, `string`, and `event` do not parse. The compilable
  surface uses `drawUint256`, `drawAddress`, etc., and `recordEvent`.
- revm 43.0.2 has `disable_nonce_check` but no `disable_balance_check`. The harness
  funds its caller explicitly.
- Solidity checks that a target has code before some void-return external calls.
  The harness installs a one-byte sentinel at the Hegel address; the inspector still
  short-circuits every call, so the sentinel never executes and draws cost no EVM gas.
- `hegel_target` requires a label in this ABI. The Solidity `target(int256)` method
  uses the stable label `hegel.sol.target`.
- libhegel may return `HEGEL_E_STOP_TEST` while shrinking even for a one-draw test;
  the frontend must consistently mark those attempts as `OVERRUN`.

For the small spike, cloning the post-deployment `CacheDB` for each case is simple and
fast enough. No journal-discard optimization is warranted yet.

# Phases 1–3 results

Completed on 2026-09-15.

- ABI auto-draw recursively covers scalars, dynamic and fixed arrays, and tuples. Named
  and auto-named Solidity draws share the same engine stream.
- Stateful contracts are discovered from `rule_*` and `invariant_*`. The runner checks
  initial/final invariants unconditionally, samples join points through the Hegel state
  machine, rejects assumed-away rules without consuming a step, and spans each rule so
  shrinking removes whole calls. The counter sample shrinks to `mint(1), burn(1)`.
- Pools keep Hegel's stable variable ids separate from Solidity `bytes32` values. Empty
  picks become rejected assumptions; consuming picks remove the host value.
- The compatibility inspector handles the selected Foundry VM calls in-process. `warp`
  and `roll` update the active revm block, `deal` journals its balance change, pranks
  rewrite the next call frame, and expected reverts are resolved in `call_end`.
- Failure replay now decodes ABI custom errors and captures common `console.log`
  overloads. JSON includes the same console and trace data as text reports.
- A native Foundry fork was evaluated but not retained: it would duplicate the working
  revm executor and require continuous synchronization with Foundry internals. The
  magic-address shim gives this individual project a stable integration boundary.

Ecosystem/community validation remains deferred.

# Private-suite scaling

Implemented on 2026-09-16.

- Independent tests can run concurrently with deterministic report ordering.
- Stable test-name hashing supports disjoint CI shards without a central manifest.
- Dynamic ABI array and byte/string limits are configurable instead of hard-coded.
- Artifact discovery follows Foundry's current cache. This fixed a concrete issue where
  a renamed contract remained under `out/` and was still discovered as a test.
- Stateful rule and invariant failures now decode ABI custom errors.
- Solidity-backed integration tests cover bounded generation, parallel scheduling,
  sharding, stale-artifact exclusion, and stateful diagnostics.
- Selected contracts are now deployed once per project run. Stateless functions and the
  stateful machine share the same immutable post-deployment snapshot while each generated
  case still clones isolated state.
- List mode reuses filtering and stable shard assignment without deploying contracts.
- Fail-fast scheduling works in sequential and parallel modes; already-running workers
  complete normally so no thread is forcibly cancelled.
- JSON reports include per-test milliseconds and text mode prints a wall-clock summary.

External ecosystem and community validation remains deferred in favor of scaling this
project's own test workload.


# Failure diagnostics and report schema

Implemented on 2026-09-17.

- Foundry artifact loading now retains deployed bytecode source maps and resolves their
  source IDs through the matching build-info file.
- The inspector records the top-level test contract's failing program counter. Replayed
  failures report a project-relative Solidity path with one-based line and column; nested
  calls still map to the top-level call/assertion that propagated the failure.
- Execution JSON now has a versioned top-level envelope, aggregate status counts, elapsed
  time, and test records. `REPORTING.md` defines the version-1 compatibility policy.


# Workload measurements

Measured on 2026-09-17 with the release binary and seed 17, excluding the one-time release
build:

- 1,000 stateless cases with up to 64 `uint256` array elements and 256 bytes completed in
  274 ms wall time (74 ms reported for the test job).
- 100 stateful cases at 100 steps each completed in 135 ms wall time (89 ms reported for
  the test job).

The project now keeps these workloads in `Scaling.t.sol` and exposes the repeatable
`scripts/benchmark.sh` driver. At these sizes EVM execution and per-case `CacheDB` clones
are not a practical bottleneck, so the clone-based isolation remains unchanged.

# Decoupling and search quality

Implemented on 2026-09-19.

## Toolchain seams

- Artifact loading sits behind `artifacts::Project`. Foundry's build cache is still the
  authoritative list of current artifacts, but it is an internal file with no
  compatibility guarantee, so an unreadable cache now degrades to scanning the artifact
  directory with a warning instead of failing the run. Build-info and source files are
  loaded per-file: a missing one costs source positions for that file alone.
- Source mapping moved to `sourcemap::SourceResolver` and answers with `Option`. Three
  degradations are covered by tests: a malformed source-map entry truncates the map, an
  unknown source id resolves to nothing, and `NoSources` handles a project with no debug
  information.
- The host-call protocol moved to one selector table in `protocol`, decoded by
  `alloy-dyn-abi`. This removed the hand-rolled ABI reader and writer, and replaced a
  chain that hashed up to twenty-four signatures per intercepted call with a single
  startup hash. The `console.log` overload set grew from five to twenty-seven, since
  adding one is now a table row.
- `Hegel.sol` carries `HEGEL_PROTOCOL_VERSION` and exposes `hegelProtocolVersion()`. The
  runner compares it before deploying any test, so a project holding an older copy is
  told which file to update rather than hitting an unknown selector mid-run.

## Search quality

- The inspector records basic blocks entered, keyed by contract and offset, and the
  runner reports the count to `hegel_target`. Collection is limited to `JUMPDEST`; the
  stateful benchmark measured 273 ms with it and 275 ms without, so it stays on by
  default (`--no-coverage-target` disables it).
- Stateful handlers are called from several accounts (`--actors`, default 3), drawn
  inside the handler's span so shrinking removes the choice with the call. Reported
  counterexamples print the sender as `vm.prank(...)`, keeping them pasteable.
- `--allow-rule-reverts` treats a reverting handler as a rejected step, which is what
  Foundry's invariant runner does by default and what a ported suite needs.

## Corrections to earlier behaviour

- `--match-test` matched the literal string "stateful" as a substring, so a pattern as
  short as `a` selected every state machine in the project. A state machine is now
  selected by naming one of its rules or invariants, or by the exact word `stateful`.
- A reverting `setUp()` was reported as a property failure, attributing a broken fixture
  to the test. It is now a hard error naming the contract and the revert reason.
- A reproduce blob applied to a selection of more than one test replayed those draws
  into whatever they happened to produce, usually reported as passes. That is now
  refused.
- `hegel.note` reached the engine but never the report. `target` now takes a label, so
  unrelated scores no longer share one objective.
- Generated strings took two different paths: ABI-drawn arguments were mapped to
  printable ASCII while `hegel.drawString` returned raw bytes rendered lossily. Both now
  use the printable mapping, so a reported string can be pasted back into a test.

## Test suite

Fixtures whose failure modes abort a run — a reverting `setUp`, a stale protocol
version — live in a separate `fixtures/` Foundry project, so `contracts/` stays runnable
end to end as the showcase suite.

Removed a benchmark that ran as a test: it asserted only that large generated
collections and deep state machines "passed", with Solidity assertions that either
mirrored the CLI flags the test itself passed or could not fail. `scripts/benchmark.sh`
already drives those workloads at larger sizes. Several other assertions were moved to
the behaviour boundary: the source-location test now reads the reported line back from
the file instead of pinning a line and column that move whenever anything above them is
edited, the sharding test uses discovery rather than executing a workload it then
ignored, and the pool test asserts that a picked value was one some handler added rather
than that the first pool id happens to be zero.
