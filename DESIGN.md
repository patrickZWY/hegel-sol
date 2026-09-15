# hegel-sol design notes

Property-based testing for Solidity, built on Hegel (Hypothesis). Status: research done,
no runner code written yet. Research date: 2026-09-15.

## Core constraint

Solidity runs inside the EVM and cannot link libhegel. The "binding" is therefore a host
runner (Rust, revm) that executes Solidity test contracts and translates draws into
libhegel calls. Every other Hegel library (Rust, Go, TS, Java, OCaml, C++) is a thin
frontend over the same C ABI, so this is the same shape, with the EVM as the "language".

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

## Planned Solidity surface

Magic address `HEGEL = address(uint160(uint256(keccak256("hegel.sol"))))`, interface
`IHegel` with: `uint256(string name, uint256 lo, uint256 hi)`, `int256(...)`, `bool(name)`,
`address(name)`, `bytes32(name)`, `bytes(name, minLen, maxLen)`, `string(name, maxLen)`,
`assume(bool)`, `startSpan(string)`, `stopSpan(bool discard)`, `note(string)`,
`target(int256 score)`, `event(string label)`. A `HegelTest` base contract exposes
`hegel` as `IHegel(HEGEL)`. Unnamed overloads can come later.

Test discovery: functions prefixed `test` (draws in body, or ABI-typed args auto-drawn
from the function signature), optional `setUp()`. Stateful: `rule_*` handlers plus
`invariant_*` checks, mapped onto the engine state machine.

Compile with `forge build` and read `out/<File>.sol/<Contract>.json` (abi + bytecode).

## Phases

0. Spike: runner with one ERC20 property, draws via magic address, confirm shrinking and
   blob replay.
1. CLI (`hegel-sol test`), settings flags, ABI auto-draw, struct/array generation.
2. Stateful machines, pools, spans, targeting.
3. Foundry integration (forge flag or drop-in shim for unchanged forge-std tests).
4. hegel-zoo entry (OpenZeppelin, Solady, Uniswap v2) and Antithesis wiring.
