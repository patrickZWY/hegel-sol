# hegel-sol design

How the runner is built and why. The API facts below were gathered on 2026-09-15 against
the versions named and are what the code relies on; the decision log at the end records
what was decided since, with dates, so a later reader can tell a constraint from a habit.

## Core constraint

Solidity runs inside the EVM and cannot link libhegel. The "binding" is therefore a host
runner (Rust, revm) that executes Solidity test contracts and translates draws into
libhegel calls. Every other Hegel library (Rust, Go, TS, Java, OCaml, C++) is a thin
frontend over the same C ABI, so this is the same shape, with the EVM as the "language".

## Layering

The crate is layered so that each volatile dependency is known in one place:

1. `engine`: safe wrappers over the libhegel C ABI. Knows nothing about the EVM.
2. `evm`, `inspector`, `protocol`: an EVM host. Intercepts calls to the Hegel and
   Foundry cheatcode addresses and knows nothing about Solidity source.
3. `artifacts`, `sourcemap`: the Foundry and solc adapters. Build a project, read its
   artifacts, and decode its debug output.
4. `runner`: discovery, scheduling, replay, and reporting. `main` is the CLI over it.

A frontend for another EVM language would replace only layers 3 and 4.

### Toolchain seams

Solidity the language moves slowly; the toolchain around it does not. The volatile
inputs are Foundry's internal build cache, solc's artifact and source-map encodings,
and Foundry's cheatcode surface, none of which carry compatibility guarantees. Three
boundaries isolate them:

- `artifacts::Project` is where a build tool is known. `FoundryProject` runs `forge
  build` and reads Foundry's cache; when that cache is missing or unparsable the loader
  warns and scans the artifact directory instead. Losing the cache costs stale-artifact
  exclusion, not the ability to run.
- `sourcemap::SourceResolver` is where solc's debug output is decoded, and it answers
  with `Option`. A malformed source-map entry truncates the map, an unreadable source
  file costs that file's line numbers, and `NoSources` covers a project with no debug
  information at all. A format change costs precision, never a run.
- `protocol` is the host-call wire format: one table mapping a selector to an operation
  and its parameter types, decoded by `alloy-dyn-abi`. Selectors are hashed once rather
  than per comparison per call, and the crate holds no second implementation of the ABI
  specification. `PROTOCOL_VERSION` is checked against the project's `Hegel.sol` before
  any test runs.

`discovery::Conventions` holds the name prefixes that decide what a test is, so the
convention is a value rather than scattered literals.

## Engine facts (hegel-rust repo, `hegel-c/include/hegel.h`, 2686 lines)

- Engine is pure Rust: crate `hegeltest-c` 0.42.3 (lib name `hegel_c`, crate-type
  cdylib + staticlib + rlib). Depending on it as an rlib gives direct `hegel_c::hegel_*`
  functions, no dlopen needed. This is what `hegeltest`'s `static-engine` feature does.
- The high-level `hegeltest` crate 0.45.4 is NOT sufficient: its big-integer path is
  `pub(crate)` and capped at 17 bytes (u128). Bind the C ABI directly.
- `hegel_generate_integer_big(ctx, tc, min, min_len, max, max_len, out, out_cap, out_len)`
  takes two's-complement little-endian signed byte buffers. For uint256 pass 33-byte
  buffers (32 bytes + zero sign byte) so max values are not read as negative.
- Other draws: `hegel_generate_boolean(ctx, tc, p, forced, has_forced, out)`,
  `hegel_generate_integer(ctx, tc, i64 min, i64 max, out)`,
  `hegel_generate_bytes(ctx, tc, min_size, max_size, out_result)` then
  `hegel_generate_bytes_result_free`. Strings via `hegel_string_generator_text` etc.
- Draws return `HEGEL_OK` or `HEGEL_E_STOP_TEST` (choice budget exhausted, report
  `HEGEL_STATUS_OVERRUN`). Assume failure: report `HEGEL_STATUS_INVALID`.
- Run lifecycle: `hegel_context_new` -> `hegel_settings_new` (+ setters:
  `set_test_cases`, `set_seed(seed, has_seed)`, `set_database(path | "" | NULL)`,
  `set_database_key`, `set_phases(mask)`, `set_print_blob`, `set_test_location`) ->
  `hegel_run_start(ctx, settings, output_cb | NULL, user_data, &run)` -> loop
  `hegel_next_test_case(ctx, run, &tc)` until tc == NULL -> run body ->
  `hegel_mark_complete(ctx, tc, status, origin)` (origin string groups failures; use
  the revert selector / assertion location) -> `hegel_test_case_free` ->
  `hegel_run_result` -> `hegel_run_result_status`, `failure_count`, `failure(i)` ->
  `hegel_failure_reproduction_blob`, `hegel_failure_origin`.
- Minimal example printing: the engine does not hand back values. Replay the blob with
  `hegel_test_case_from_blob(ctx, settings, blob, cb, ud, &tc)`, rerun the EVM test
  recording every draw, print the trace and revert reason. This is what hegeltest does.
- Spans: `hegel_label_from_name`, `hegel_start_span(ctx, tc, label)`,
  `hegel_stop_span(ctx, tc, discard)`. Wrap each handler call in stateful tests so the
  shrinker can delete whole calls.
- Stateful: `hegel_new_state_machine(ctx, tc, rule_names, rule_groups, n_rules,
  invariant_names, invariant_always_check, n_inv, min_conc=1, max_conc=1, step_count=50,
  &sm, &conc)` then rounds of `hegel_state_machine_next_group` /
  `hegel_state_machine_next_rule(worker=0)` / `hegel_state_machine_rule_rejected` /
  `hegel_state_machine_should_check_invariant`. Pools: `hegel_new_pool`, `hegel_pool_add`,
  `hegel_pool_generate(consume)` for addresses and ids created at runtime.
- Targeting: `hegel_target(score)`; events: `hegel_event`, `hegel_event_value`.
- Profiles: `hegel.toml` in cwd or ancestors, `HEGEL_DEFAULT_PROFILE`, shipped profiles
  `development`, `ci`, `workload` (Antithesis).

## revm 43.0.2 facts (registry sources)

- Interception: implement `revm::Inspector<CTX>` and override
  `fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome>`.
  Returning `Some(outcome)` short-circuits the call. Match on `inputs.target_address ==
  HEGEL_ADDRESS`, read calldata with `inputs.input.bytes(ctx)`, build
  `CallOutcome::new(InterpreterResult::new(InstructionResult::Return, output, Gas::new(inputs.gas_limit)), inputs.return_memory_offset.clone())`.
  Use `InstructionResult::Revert` to abort on assume-failure or STOP_TEST and set a flag
  on the inspector so the runner classifies the case afterwards.
- Build: `Context::mainnet().with_db(CacheDB::new(EmptyDB::default())).modify_cfg_chained(|c| c.disable_nonce_check = true).build_mainnet_with_inspector(insp)`;
  run with `InspectEvm::inspect_one_tx(TxEnv)`; `TxEnv::builder().caller(..).kind(TxKind::Call(addr)).data(..).gas_limit(..).build_fill()`.
  Result: `ExecutionResult::{Success{output,..}, Revert{output,..}, Halt{reason,..}}`.
- Deploy via `TxKind::Create` with bytecode, or insert code directly with
  `CacheDB::insert_account_info(addr, AccountInfo{..})` + `Bytecode::new_raw`.
- Snapshot per test case: clone the `CacheDB` (cheap for small state) or use
  `inspect_tx` without commit so the journal is discarded.
- Use `revm::primitives` (alloy-primitives 1.7.3) for Address/U256/Bytes to avoid
  version splits with `alloy-json-abi` / `alloy-dyn-abi` 1.7.x.
- revm 43.0.2 has `disable_nonce_check` but no `disable_balance_check`. The harness
  funds its caller explicitly.

## Solidity surface

`solidity/src/Hegel.sol` is the reference: a magic address
`HEGEL = address(uint160(uint256(keccak256("hegel.sol"))))`, the `IHegel` interface,
the `IHegelVm` subset of Foundry cheatcodes, and a `HegelTest` base contract exposing
both. Test discovery follows Foundry's conventions: `test*` functions with draws in the
body or ABI-typed arguments generated from the signature, optional `setUp()`, and
`rule_*` handlers plus `invariant_*` checks mapped onto the engine state machine.

Constraints the language imposed on that surface:

- Solidity reserves elementary type names and `event`, so methods named `uint256`,
  `address`, `bool`, `bytes`, `string`, and `event` do not parse. The surface uses
  `drawUint256`, `drawAddress`, and so on, and `recordEvent`.
- Solidity checks that a target has code before some void-return external calls. The
  harness installs a one-byte sentinel at the Hegel address; the inspector still
  short-circuits every call, so the sentinel never executes and draws cost no EVM gas.
- `hegel_target` requires a label in this ABI. The unlabelled Solidity `target(int256)`
  uses the stable label `hegel.sol.target`.

## Coverage-guided generation

The inspector records the basic blocks an example enters, keyed by contract address and
offset, and the runner reports the count to `hegel_target`. Collection is limited to
`JUMPDEST` so the per-opcode hook stays cheap; the stateful benchmark measured 273 ms
with it and 275 ms without. `--no-coverage-target` disables it.

## Decision log

### 2026-09-15: the loop works end to end

With seed 1 and 50 cases the ERC20 example finds the deliberate supply bug and shrinks it
to `from = 0x00..00`, `to = 0x00..01`, `amount = 3` in about 50 ms after compilation,
and the emitted blob replays the same draw trace. libhegel may return
`HEGEL_E_STOP_TEST` while shrinking even a one-draw test; the frontend marks those
attempts `OVERRUN` consistently.

Per-case isolation clones the post-deployment `CacheDB`. Measured later at 1,000
stateless cases with 64-element arrays and 256-byte payloads in 274 ms wall time, and
100 stateful cases of 100 steps in 135 ms, so a journal-discard optimisation is not
warranted.

### 2026-09-15: standalone runner, not a Foundry fork

A `forge test --fuzzer hegel` fork was evaluated and rejected. It would duplicate the
working revm executor and couple this project to Foundry's internal executor APIs, which
carry no compatibility guarantee, without adding engine capability. The magic-address
shim is the stable integration boundary. Native upstream integration remains an optional
future distribution path.

### 2026-09-16: scaling a private suite

- Independent stateless tests and stateful contracts run concurrently under `--jobs`,
  with reports kept in discovery order.
- `--shard NUMBER/TOTAL` partitions tests by a stable hash of the test name, so shards
  need no central manifest.
- Artifact discovery follows Foundry's compilation cache. This fixed a renamed contract
  whose stale JSON under `out/` was still discovered as a test.
- Each selected contract is deployed once; its post-deployment snapshot and `setUp()`
  selector are shared by all of its test jobs.
- `--fail-fast` stops sequential runs immediately and stops parallel workers from
  claiming new jobs; running workers finish normally, so nothing is cancelled mid-EVM.

### 2026-09-17: diagnostics and the report contract

Failure replays resolve the top-level revert or halt program counter through the
deployed source map and report a project-relative path, line, and column. Nested calls
map to the top-level call or assertion that propagated the failure. The JSON report
gained a versioned envelope; `docs/reporting.md` states the compatibility rules.

A benchmark that ran as a test was removed: it asserted only that large workloads
"passed", with Solidity assertions that mirrored the CLI flags the test itself passed.
Workloads live in `benchmarks/` and are driven by `cargo x bench`.

### 2026-09-19: decoupling and search quality

The toolchain seams above were introduced. Moving the host-call protocol into one
selector table removed a hand-rolled ABI codec and a chain that hashed up to
twenty-four signatures per intercepted call; the `console.log` overload set grew from
five to twenty-seven because adding one became a table row.

Search quality: basic-block coverage is reported as a target score; handlers are called
from several accounts, drawn inside the handler's span so shrinking removes the choice
with the call, and reported as `vm.prank(...)` so counterexamples stay pasteable;
`--allow-rule-reverts` matches Foundry's default invariant policy for ported suites.

Corrections to earlier behaviour:

- `--match-test` matched the literal string "stateful" as a substring, so a pattern as
  short as `a` selected every state machine. A machine is now selected by naming one of
  its rules or invariants, or by the exact word `stateful`.
- A reverting `setUp()` was reported as a property failure, once per example. It is now
  a hard error naming the contract and the revert reason.
- A reproduce blob applied to a selection of more than one test replayed its draws into
  whatever they produced, usually reported as passes. That is refused.
- `hegel.note` reached the engine but never the report. `target` now takes a label so
  unrelated scores do not share one objective.
- ABI-drawn strings were mapped to printable ASCII while `drawString` returned raw bytes
  rendered lossily. Both use the printable mapping, so a reported string pastes back.

Test-suite boundaries: the source-location test reads the reported line back from the
file rather than pinning a position; the sharding test uses discovery; the pool test
asserts that a picked value was one some handler added.

### 2026-09-23: repository layout by audience

`contracts/` had been serving as showcase, integration-test target, and benchmark
workload at once, with the product's own `Hegel.sol` buried inside it. The repository
now has one directory per audience: `solidity/` for the file users install, `examples/`
for the showcase, `benchmarks/` for workloads, and Foundry projects under
`crates/tests-integration/projects/` for behaviour fixtures and for projects the runner
must refuse. Shell scripts became `cargo x` subcommands in `crates/xtask`, which
discovers Foundry projects by their `foundry.toml`. The dev-journal documents were folded
into this log; `AGENTS.md` holds contributor conventions; `CHANGELOG.md` tracks
user-visible change. `--root` now defaults to the current directory, since the old
default named a directory of this repository.

### 2026-09-29: validation before the first release

The synthetic workloads in [`benchmarks/`](benchmarks/) measure scaling, but they do
not establish compatibility or bug-finding value on real contracts. The first release
needs a repeatable comparison on pinned third-party code. The plan below guides that
work; the [first ecosystem pass](crates/tests-integration/projects/real-world/README.md)
records initial results and remaining measurement gaps.

1. **Make runs reproducible.** Record the runner commit, upstream commit, solc and
   Foundry versions, Rust version, machine CPU and memory, exact command, seed, case
   count, step count, actor count, job count, and database setting. Keep upstream
   checkouts outside the workspace so `cargo x` does not discover them as repository
   fixtures. Keep pinned source and property harnesses under
   `crates/tests-integration/projects/real-world/`; put regression tests for runner
   bugs under `crates/tests-integration/projects/behaviour/`. Run external properties
   through `cargo x validate` so the repository workflow owns their toolchain steps.
2. **Establish correctness on real code.** Start with one ERC20 property and one
   stateful property each for [OpenZeppelin Contracts][openzeppelin] and
   [Solady][solady]. Then add a reserve/accounting invariant over
   [Uniswap v2 core][uniswap]. Prefer an
   existing Foundry test when its cheatcodes are supported; otherwise write a small
   Foundry harness importing the pinned upstream contracts. Run the same assertion
   under Forge first. Record each test as passed, reproducible failure, unsupported
   feature, or harness/setup error. Replay every Hegel failure from a fresh checkout
   and confirm the underlying assertion with Forge or a minimal Solidity test before
   calling it a contract bug. Keep a minimized trace and source location for each
   failure, including false positives.
3. **Measure search and runtime separately.** On those same harnesses, compare Forge
   and hegel-sol with matched seeds where supported, case budgets, stateful depth, and
   actor assumptions; document any mismatch that prevents a fair comparison. Report
   time to first confirmed failure or coverage reached at a fixed budget, rather than
   comparing pass counts. Run cold and warm compilation separately from execution.
   For hegel-sol, measure wall time, peak RSS, cases per second, invalid/overrun rate,
   shrink time, replay time, and basic-block coverage. Run at least five repetitions
   per configuration and report median and range. Use `--database off` for fresh-search
   comparisons and a separate warm-database run for regression replay.
4. **Profile before optimizing.** First repeat `cargo x bench` in release mode and
   extend it with a job-count option before measuring one job against parallel jobs.
   Profile representative stateless and stateful real-world harnesses using a sampling
   profiler, retaining its output and exact run metadata. Attribute time to
   compilation/artifact loading, EVM execution,
   engine generation, coverage collection, shrinking, and report rendering. Change
   only a measured bottleneck; rerun the same pinned workload and compare median time
   and peak RSS before describing an improvement.
5. **Publish an evidence table.** For each upstream revision and property, record
   selection/discovery, support status, observed failures, confirmed bugs, false
   positives, coverage, runtime, and reproduction artifact. Link any runner fixes to
   a focused integration test. Release readiness requires all selected properties to
   either run and replay deterministically or have a documented compatibility limit;
   every reported contract bug must reproduce independently. Publish performance
   numbers only with the workload, environment, and variance attached. If these
   results hold, contribute a `solidity/` entry to hegel-zoo.

[openzeppelin]: https://github.com/OpenZeppelin/openzeppelin-contracts
[solady]: https://github.com/Vectorized/solady
[uniswap]: https://github.com/Uniswap/v2-core

## Deferred work

- Extend ecosystem validation to more properties, failing cases with independently
  confirmed reproduction, coverage measurements, and matched stateful baselines.
- Attribute CPU time with a sampling profiler on a host that permits it; add
  multi-job real-world workloads before evaluating parallel scaling.
- Antithesis: set `hegel_settings_set_test_location` per test and run under the
  `workload` profile.
- Compatibility growth: extend cheatcodes, console overloads, forked-state support, and
  environment controls when real suites require them.
