# hegel-sol

Property-based and stateful testing for Solidity, built on
[Hegel](https://hegel.dev) and [revm](https://github.com/bluealloy/revm).

The standalone runner supports Solidity-side draws, ABI auto-draw for unchanged fuzz
tests, shrinking and replay blobs, persistent example databases, rule/invariant state
machines, variable pools, targeting and events, JSON reports, custom errors,
`console.log`, and a focused Foundry cheatcode compatibility layer.

## Demo

Run the deterministic three-part demonstration:

```sh
./scripts/demo.sh
```

It shows stateless shrinking, stateful sequence shrinking, and decoded Solidity
diagnostics. See [DEMO.md](DEMO.md) for the narrated walkthrough.

## Try it

```sh
cargo run -p hegel-sol -- test \
  --root contracts \
  --match-test transfer \
  --seed 1 \
  --test-cases 50 \
  --database off
```

The deliberately buggy token shrinks to `from = address(0)`, `to = address(1)`, and
`amount = 3`. Replay a printed blob with:

```sh
HEGEL_SOL_REPRODUCE='<blob>' cargo run -p hegel-sol -- test --root contracts
```

Use `--help` for contract/test filters, profiles, phases, database selection, stateful
step count, statistics, JSON, verbosity, and scaling controls.

## Scaling a private test suite

Run independent tests concurrently, split them deterministically across CI workers, and
raise or lower generated collection sizes:

```sh
# Local parallelism; reports remain in deterministic discovery order.
hegel-sol test --root contracts --jobs 8

# Two CI workers running stable, disjoint subsets.
hegel-sol test --root contracts --shard 1/2
hegel-sol test --root contracts --shard 2/2

# Explore larger dynamic values when the contract needs them.
hegel-sol test --root contracts --max-array-len 32 --max-byte-len 256

# Inspect discovery or shard placement without running tests.
hegel-sol test --root contracts --shard 1/2 --list

# Stop scheduling after a failure and show per-test durations.
hegel-sol test --root contracts --jobs 8 --fail-fast --show-timings
```

Sharding happens per discovered stateless test or stateful contract. Artifact discovery
uses Foundry's current compilation cache, so contracts left behind in `out/` after a
rename or deletion are not accidentally executed.

Selected contracts are deployed once and their clean post-deployment snapshots are
shared across independent test jobs. `--fail-fast` prevents new work from being
scheduled after a failure; tests already running in other workers are allowed to finish.
JSON reports include `duration_ms`, while text output always ends with a wall-clock
status summary.

## Solidity API

Inherit `HegelTest`, then draw through `hegel`:

```solidity
import {HegelTest} from "../src/Hegel.sol";

contract TokenTest is HegelTest {
    function test_transfer() public {
        address from = hegel.drawAddress("from");
        address to = hegel.drawAddress(); // unnamed draws become draw_0, draw_1, ...
        hegel.assume(from != to);
        uint256 amount = hegel.drawUint256("amount", 0, 100);
        // exercise the system and assert a property
    }
}
```

Solidity reserves names such as `uint256`, `address`, and `event`, so the API uses
`drawUint256`, `drawAddress`, and `recordEvent`.

## Stateful tests

Functions prefixed with `rule_` are handlers; functions prefixed with `invariant_` are
checked initially, at sampled join points, and finally. `stepCount()` can override the
CLI's default state-machine length.

```solidity
contract CounterTest is HegelTest {
    uint256 value;

    function rule_add(uint8 amount) public { value += amount; }
    function invariant_bounded() public view { assert(value < 1_000); }
    function stepCount() public pure returns (uint256) { return 25; }
}
```

`poolNew`, `poolAdd`, and `poolPick` let handlers shrink references to values created by
earlier rules. The runner reports the minimized rule sequence and auto-drawn arguments.

## Foundry compatibility

The magic `vm` address supports `prank`, `startPrank`, `stopPrank`, `deal`, `warp`,
`roll`, `expectRevert`, `label`, and `assume`. This is enough for small
OpenZeppelin-style tests that use those cheatcodes. Calls to Foundry's standard console
address are captured, and ABI custom errors are decoded in failures.

This project deliberately remains a standalone runner instead of maintaining a Foundry
fork. A future native `forge test --fuzzer hegel` integration would require ongoing
coupling to Foundry's internal fuzz and invariant executors.

## Layout

- `crates/hegel-sol`: safe Hegel FFI wrappers, artifact loader, revm harness,
  magic-address inspector, stateful/stateless run loops, reports, and CLI.
- `contracts/src/Hegel.sol`: draws, pools, and supported VM cheatcode interfaces.
- `contracts/test`: shrinking, auto-draw, stateful, pool, and compatibility examples.
- `DESIGN.md`, `PLAN.md`, and `NOTES.md`: architecture, status, and implementation notes.

Phases 0–3 and the first private-suite scaling milestone are implemented. Broader
ecosystem/community work is intentionally deferred while this remains an individual
project.
