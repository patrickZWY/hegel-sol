# hegel-sol implementation plan

> Implementation status (2026-09-15): Phases 0–3 are complete in the standalone runner.
> Phase 4 ecosystem/community validation is intentionally deferred for this individual project.

Property-based testing for Solidity, built on Hegel (Hypothesis engine, libhegel C ABI).
Read `DESIGN.md` first: it records the exact engine and revm API facts gathered on
2026-09-15 so no step below needs re-research.

Each step lists its deliverable and a concrete "done when" check. Work top to bottom.
Steps inside a phase are ordered; phases are sequential.

---

## Phase 0: spike (prove the loop works end to end)

Goal: one Solidity property test, draws made from Solidity, engine shrinks a failure to a
minimal example and prints a reproduce blob that replays.

### 0.1 Toolchain
- Install Rust stable (rustup, `--profile minimal`), Foundry (`forge` 1.3.x).
- Done when: `cargo --version` and `forge --version` both work.

### 0.2 FFI layer: `crates/hegel-sol/src/engine/`
- Depend on `hegeltest-c = "=0.42.3"` (rlib, exposes `hegel_c::hegel_*`).
- Write safe wrappers, each owning its raw pointer and freeing in `Drop`:
  `Context`, `Settings`, `Run`, `TestCase`, `RunResult`, `Failure`.
- `TestCase` methods: `integer_big(min: &[u8], max: &[u8]) -> Result<Vec<u8>, Stop>`,
  `integer(i64, i64)`, `boolean(p)`, `bytes(min, max)`, `start_span(label)`,
  `stop_span(discard)`, `note(str)`, `target(f64)`, `event(label)`,
  `mark_complete(status, origin)`.
- Map `HEGEL_E_STOP_TEST` to a `Stop` error; every other negative code is a hard error
  with the message from `hegel_context_last_error`.
- Done when: a Rust unit test runs a fake property (draw an i64, fail if > 10) through
  `Run` and the result reports one failure with a non-null blob.

### 0.3 Integer encoding helpers
- `u256_to_le_signed(U256) -> [u8; 33]` (32 LE bytes + zero sign byte) and the inverse.
- `i256_to_le_signed` for signed draws (33 bytes, sign-filled).
- Done when: round-trip unit tests pass for 0, 1, 2^255, 2^256-1, -1, i256::MIN.

### 0.4 Solidity side: `contracts/src/Hegel.sol`
- `address constant HEGEL = address(uint160(uint256(keccak256("hegel.sol"))))`.
- `interface IHegel` with named draws:
  `uint256(string name, uint256 lo, uint256 hi) returns (uint256)`,
  `int256(string, int256, int256)`, `bool(string)`, `address(string)`,
  `bytes32(string)`, `bytes(string, uint256 minLen, uint256 maxLen)`,
  `string(string, uint256 maxLen)`, plus `assume(bool)`, `startSpan(string)`,
  `stopSpan(bool discard)`, `note(string)`, `target(int256)`, `event(string)`.
- `abstract contract HegelTest { IHegel constant hegel = IHegel(HEGEL); }`.
- Done when: `forge build` in `contracts/` succeeds.

### 0.5 Compile and load artifacts: `src/artifacts.rs`
- Run `forge build --root <dir> --extra-output abi` and read
  `out/<File>.sol/<Contract>.json`: `abi`, `bytecode.object`.
- Model: `Contract { name, abi: alloy_json_abi::JsonAbi, bytecode: Bytes, path }`.
- Done when: loading the spike test contract yields its `test*` function selectors.

### 0.6 EVM harness: `src/evm.rs`
- `Harness::new()` builds a revm `Context::mainnet()` on `CacheDB<EmptyDB>` with
  `disable_nonce_check`, a funded default caller, and a high gas limit.
- `deploy(bytecode) -> Address`, `call(to, calldata) -> ExecutionResult`.
- Snapshot per test case: keep the post-deploy `CacheDB` and clone it before each
  case (simplest correct approach for the spike).
- Done when: deploying and calling a trivial contract returns `Success`.

### 0.7 Draw interception: `src/inspector.rs`
- `HegelInspector { tc: &TestCase, trace: Vec<Draw>, outcome: Option<Abort> }`
  implementing `revm::Inspector`, overriding `call`. On `target_address == HEGEL`:
  decode selector, dispatch to the engine, ABI-encode the value, return
  `CallOutcome` with `InstructionResult::Return`.
- On assume(false): set `outcome = Invalid`, return `Revert`.
- On `Stop` from the engine: set `outcome = Overrun`, return `Revert`.
- Record every draw as `(name, type, value)` in `trace`.
- Done when: a Solidity test calling `hegel.uint256("x", 0, 100)` receives a value in
  range.

### 0.8 Run loop: `src/runner.rs`
- For each test contract and each `test*` function: build `Settings`
  (database key = `<Contract>.<function>`), start a `Run`, loop
  `next_test_case`, per case: clone DB, run `setUp()` if present, call the test
  function under the inspector, classify:
  inspector Invalid -> `INVALID`; Overrun -> `OVERRUN`; EVM `Revert`/`Halt` ->
  `INTERESTING` with origin = decoded revert reason or 4-byte selector, else panic
  code; `Success` -> `VALID`.
- After the run: for each failure, replay the blob with `test_case_from_blob`, rerun the
  case, and print the recorded draw trace plus the revert reason and the blob.
- Done when: the spike property below fails, prints a minimal counterexample, and
  `HEGEL_SOL_REPRODUCE=<blob>` replays it.

### 0.9 Spike property: `contracts/test/ERC20Spike.t.sol`
- Minimal ERC20 with a deliberate bug (e.g. `transfer` does not check balance when
  `amount == balance + 1`). Property: draw two addresses and an amount, transfer,
  assert `sum of balances == totalSupply`.
- Done when: hegel-sol finds it and shrinks to the smallest amount that triggers it.

### 0.10 Write up spike results in `NOTES.md`
- Shrink quality observed, run time per case, anything in the engine or revm API that
  differed from `DESIGN.md`.

---

## Phase 1: usable CLI — complete

### 1.1 `hegel-sol test` CLI (clap)
- Flags: `--root`, `--match-contract`, `--match-test`, `--test-cases N`, `--seed`,
  `--database <path|off>`, `--phases`, `--profile`, `--reproduce <blob>`,
  `--verbosity`, `--json`.
- Respect `hegel.toml` profiles via `hegel_settings_new_for_profile`.
- Done when: `hegel-sol test --root contracts --match-test transfer` runs only that test.

### 1.2 ABI auto-draw
- `testFuzz_x(uint256 a, address b, bytes calldata c)` with no draws in the body:
  generate each argument from its ABI type before the call, inside a span per argument.
- Support: all `uintN`/`intN`, `bool`, `address`, `bytesN`, `bytes`, `string`, fixed and
  dynamic arrays, tuples/structs (recursive, via `alloy_dyn_abi::DynSolType`).
- Done when: an unchanged forge-std style fuzz test runs under hegel-sol.

### 1.3 Unnamed draw overloads and `Hegel.sol` polish
- Unnamed overloads that auto-name draws `draw_<n>`. NatSpec on every function.
- Done when: both named and unnamed forms compile and print sensibly.

### 1.4 Reporting
- Pretty-print the minimal example as Solidity-ish assignments
  (`uint256 amount = 3;`), decode custom errors from the ABI, show `console.log`
  output captured via the standard console address (0x000...636F6e736F6c652e6c6f67).
- Emit an `--json` report (test, status, origin, blob, trace).
- Done when: output matches the style of hegel-rust failure reports.

### 1.5 Example database
- Default `.hegel/examples/` in the project root; key per contract+function.
- Done when: a second run reuses the stored failure without re-searching (REUSE phase).

### 1.6 CI
- GitHub Actions: build, `cargo test`, run the sample contracts with `--profile ci`.

---

## Phase 2: stateful testing — complete

### 2.1 Rule and invariant discovery
- Test contract exposes `rule_*` functions and `invariant_*` functions.
  Optional `stepCount()` view for `step_count`.
- Done when: discovery lists rules and invariants for a sample contract.

### 2.2 Machine loop
- Per test case: `new_state_machine(rules, groups all 0, invariants, 1, 1, steps)`;
  rounds: `next_group` -> `next_rule(0)` -> span around the call -> call rule
  (arguments auto-drawn as in 1.2, or drawn inside the body) -> if the rule reverts with
  the assume marker call `rule_rejected` -> check invariants where
  `should_check_invariant` says so.
- Done when: a token stateful test with a mint/burn/transfer handler set runs and a
  seeded bug shrinks to a short call sequence.

### 2.3 Pools
- Solidity: `hegel.poolNew(string) returns (uint256 id)`, `poolAdd(id, bytes32)`,
  `poolPick(id, bool consume) returns (bytes32)`. Runner keeps the values; the engine
  keeps the variable ids.
- Done when: handlers can pick "a previously created account" instead of a random one.

### 2.4 Targeting and events
- Wire `hegel.target` and `hegel.event`; expose `--show-statistics`.

---

## Phase 3: Foundry integration — complete for the standalone runner

### 3.1 forge-std compatibility shim
- Implement enough `vm.*` cheatcodes to run typical forge tests: `prank`, `startPrank`,
  `stopPrank`, `deal`, `warp`, `roll`, `expectRevert`, `label`, `assume`. Map `vm.assume`
  to hegel assume.
- Done when: a small OpenZeppelin-style forge test suite runs unchanged.

### 3.2 Upstream path
- Prototype a `forge test --fuzzer hegel` fork branch that swaps forge's fuzz and
  invariant executors for the hegel-sol runner. Evaluate effort vs. the shim.

Decision: keep the revm runner and compatibility shim. A Foundry fork would couple this
individual project to unstable internal executor APIs without adding engine capability.
Native upstream integration remains an optional future distribution path.

---

## Phase 4: ecosystem/community proof — deferred

### 4.1 Zoo
- Run against OpenZeppelin Contracts, Solady, Uniswap v2. Record bugs found, false
  positives, and run time. Contribute a `solidity/` entry to hegel-zoo if results hold.

### 4.2 Antithesis
- Set `hegel_settings_set_test_location` per test and run under the `workload`
  profile; verify assertions appear in Antithesis output.

---

## Open questions to settle during Phase 0
- Do draws cost gas? Proposal: no (return full gas), so overrun is purely engine budget.
- Per-case DB clone vs. journal discard: measure once the spike runs.
- uint256 shrink quality through `integer_big` versus composing two u128 draws.
