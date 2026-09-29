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
./target/release/hegel-sol test --root examples
```

`--root` names a Foundry project and defaults to the current directory. Run the
deterministic feature demo with `cargo x demo`. Use `hegel-sol test --help` for every
option.

The runner discovers `test*` functions, runs `setUp()` before each generated case, and
auto-generates ABI arguments for ordinary Forge fuzz tests. Discovery prefixes are
configurable with `--test-prefix`, `--rule-prefix`, and `--invariant-prefix`. Its default
example database is `<root>/.hegel/examples`; disable it with `--database off`. A
reproduce blob belongs to one test, so replaying one requires a selection of exactly one.
Replay a failure with the printed command or:

```sh
HEGEL_SOL_REPRODUCE='<blob>' hegel-sol test --root examples
```

## Solidity tests

Tests import [`solidity/src/Hegel.sol`](solidity/src/Hegel.sol) when they need explicit
draws. Copy the file into your project, or install this repository as a Foundry
dependency and remap it:

```toml
# foundry.toml
remappings = ["hegel-sol/=lib/hegel-sol/solidity/src/"]
```

```solidity
import {HegelTest} from "hegel-sol/Hegel.sol";

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

The [`examples/`](examples) project shows each of these in a complete, runnable test.

## Useful commands

```sh
hegel-sol test --root examples --jobs 8 --fail-fast --show-timings
hegel-sol test --root examples --shard 1/2
hegel-sol test --root examples --max-array-len 32 --max-byte-len 256
hegel-sol test --root examples --actors 5 --allow-rule-reverts
hegel-sol test --root examples --list
hegel-sol test --root examples --json
```

Generation is guided by the basic blocks each example reaches, reported to the engine as
a target score. Turn it off with `--no-coverage-target`.

Reports stay in deterministic discovery order. Shards are stable and disjoint. Selected
contracts are deployed once, while each generated case receives isolated state. Profiles
come from `hegel.toml` via `--profile`.

JSON output is versioned and documented in [docs/reporting.md](docs/reporting.md).

## Foundry compatibility

Supported `vm` calls: `prank`, `startPrank`, `stopPrank`, `deal`, `warp`, `roll`,
`expectRevert`, `label`, and `assume`. Common `console.log` calls and ABI custom errors
are decoded. Forked-state cheatcodes are not supported; this project intentionally remains
a standalone runner instead of a Foundry fork.

## Repository layout

| Path                        | Holds                                                          |
| --------------------------- | -------------------------------------------------------------- |
| `crates/hegel-sol`          | The runner: engine FFI, EVM host, Foundry and solc adapters, CLI. |
| `solidity/`                 | `Hegel.sol`, the interface and base contract test projects import. |
| `examples/`                 | The showcase Foundry project this README and the demo point at. |
| `benchmarks/`               | Workloads for `cargo x bench`.                                 |
| `crates/tests-integration/` | Integration tests and the Foundry projects they run against.   |
| `crates/xtask/`             | The `cargo x` workflows.                                       |
| `docs/`                     | Reference material such as the JSON report schema.             |

## Development

```sh
cargo x lint            # rustfmt, clippy, forge fmt, typos
cargo x build --locked  # every crate and every Foundry project
cargo x test            # unit, integration, and pinned real-contract tests
cargo x demo            # three intentional failures, found and shrunk
cargo x bench           # release-mode scaling workloads
cargo x validate        # pinned real-contract properties under Forge and hegel-sol
cargo x profile         # wall time, CPU time, and peak memory for one real workload
```

CI runs the first three on every push and pull request; `cargo x lint --fix` applies
formatting and lint fixes. Enable the committed pre-push hook, which runs the same
checks, with `git config core.hooksPath .githooks`. Pushing a `vX.Y.Z` tag runs the
checks again, builds a Linux binary, and publishes it with a checksum as a GitHub
release.

The [real-contract validation record][real-world]
pins upstream revisions and records the first run's results and limits.

[real-world]: crates/tests-integration/projects/real-world/README.md

Conventions for contributors and coding agents: [AGENTS.md](AGENTS.md). Architecture
and decisions: [DESIGN.md](DESIGN.md). User-visible changes: [CHANGELOG.md](CHANGELOG.md).
