# hegel-sol

Property-based and stateful Solidity testing with [Hegel](https://hegel.dev) and
[revm](https://github.com/bluealloy/revm). It is a standalone Foundry-compatible runner
with shrinking, replay blobs, persistent examples, parallel execution, and Solidity
source-mapped failures.

## Quick start

Requires Rust and [Foundry](https://book.getfoundry.sh/getting-started/installation).
From this repository:

```sh
cargo build --release --locked
./target/release/hegel-sol test --root contracts
```

Run the deterministic feature demo with `./scripts/demo.sh`. Use `hegel-sol test --help`
for every option.

The runner discovers `test*` functions, runs `setUp()` before each generated case, and
auto-generates ABI arguments for ordinary Forge fuzz tests. Discovery prefixes are
configurable with `--test-prefix`, `--rule-prefix`, and `--invariant-prefix`. Its default
example database is `<root>/.hegel/examples`; disable it with `--database off`. A
reproduce blob belongs to one test, so replaying one requires a selection of exactly one.
Replay a failure with the printed command or:

```sh
HEGEL_SOL_REPRODUCE='<blob>' hegel-sol test --root contracts
```

## Solidity tests

Copy or import [`contracts/src/Hegel.sol`](contracts/src/Hegel.sol) when tests need
explicit draws:

```solidity
import {HegelTest} from "../src/Hegel.sol";

contract TokenTest is HegelTest {
    function test_transfer() public {
        address from = hegel.drawAddress("from");
        address to = hegel.drawAddress("to");
        uint256 amount = hegel.drawUint256("amount", 0, 100);
        hegel.assume(from != to);
        // Exercise the system and assert a property.
    }
}
```

The API also provides signed, boolean, bytes, string, pool, target, event, note, and span
operations; its NatSpec is the complete reference. ABI auto-draw supports scalars, bytes,
strings, arrays, tuples, and structs.

`Hegel.sol` declares `HEGEL_PROTOCOL_VERSION` and exposes it through
`hegelProtocolVersion()`. The runner checks it before executing anything, so a copy that
has fallen behind is named rather than failing later as an unknown selector.

For stateful tests, name handlers `rule_*` and properties `invariant_*`. Invariants run
initially, at sampled join points, and finally. Optional `stepCount()` overrides the CLI
default. Handlers are called from several accounts (`--actors`, default 3) so invariants
needing two parties are reachable; the reported counterexample names the sender of each
call. A reverting handler fails the test unless `--allow-rule-reverts` is set, which
matches Foundry's default invariant behaviour and suits suites ported from it.

```solidity
contract CounterTest {
    uint256 value;

    function rule_add(uint8 amount) public { value += amount; }
    function invariant_bounded() public view { assert(value < 1_000); }
    function stepCount() public pure returns (uint256) { return 25; }
}
```

## Useful commands

```sh
hegel-sol test --root contracts --jobs 8 --fail-fast --show-timings
hegel-sol test --root contracts --shard 1/2
hegel-sol test --root contracts --max-array-len 32 --max-byte-len 256
hegel-sol test --root contracts --actors 5 --allow-rule-reverts
hegel-sol test --root contracts --list
hegel-sol test --root contracts --json
```

Generation is guided by the basic blocks each example reaches, reported to the engine as
a target score. Turn it off with `--no-coverage-target`.

Reports stay in deterministic discovery order. Shards are stable and disjoint. Selected
contracts are deployed once, while each generated case receives isolated state. Profiles
come from `hegel.toml` via `--profile`.

JSON output is versioned and documented in [REPORTING.md](REPORTING.md).

## Foundry compatibility

Supported `vm` calls: `prank`, `startPrank`, `stopPrank`, `deal`, `warp`, `roll`,
`expectRevert`, `label`, and `assume`. Common `console.log` calls and ABI custom errors
are decoded. Forked-state cheatcodes are not supported; this project intentionally remains
a standalone runner instead of a Foundry fork.

## Development

```sh
./scripts/ci.sh          # format, lint, build, and test
./scripts/benchmark.sh   # release-mode scaling workloads
```

`contracts/` is the showcase project. `fixtures/` holds contracts that exercise how the
runner reacts to broken projects; several abort a run by design, so they are kept
separate.

Architecture and status: [DESIGN.md](DESIGN.md), [PLAN.md](PLAN.md), and
[NOTES.md](NOTES.md).
