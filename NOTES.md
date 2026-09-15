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
