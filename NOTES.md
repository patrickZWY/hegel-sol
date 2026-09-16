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

Phase 4 ecosystem/community validation remains deferred.
