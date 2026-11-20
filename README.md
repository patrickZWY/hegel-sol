# hegel-sol

Property-based and stateful testing for Solidity, built on [Hegel](https://hegel.dev)
and [revm](https://github.com/bluealloy/revm). It is a standalone runner for Foundry
projects: it finds an input that breaks a property, shrinks it to a minimal one, prints
it as Solidity, and hands back a blob that replays it.

## Install

Requires [Foundry](https://book.getfoundry.sh/getting-started/installation). Download
the Linux binary from the [latest release][releases], or build from a checkout:

```sh
cargo install --locked --path crates/hegel-sol
```

Tests that draw values explicitly import [`Hegel.sol`](solidity/src/Hegel.sol). Copy it
into your project, or add this repository as a Foundry dependency and remap it:

```toml
# foundry.toml
remappings = ["hegel-sol/=lib/hegel-sol/solidity/src/"]
```

The runner checks the file's protocol version before running anything, so a copy that
has fallen behind is reported by name.

## Write a test

Every `test*` function is a property. Its arguments are generated from the ABI, so
ordinary Forge fuzz tests run unchanged. For explicit, named draws extend `HegelTest`:

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

`hegel` also draws signed integers, booleans, bytes, and strings, and provides spans,
notes, targets, events, and pools. The NatSpec in `Hegel.sol` is the full reference.

For inputs constrained by your fixture, `HegelTest` provides named choices:

```solidity
address from = pick("from", actors);
address to = pickOther("to", actors, from);
uint256 amount = hegel.drawUint256("amount", 0, token.balanceOf(from));
```

`pick` accepts address or integer arrays; `pickIndex` selects from your own struct
collections. Choices shrink toward earlier candidates and record the selected values
in failure notes. Empty choices report `HegelEmptyChoice` with the choice name.
[Choosing useful inputs](docs/choices.md) explains constraints, composition, and replay,
with [runnable transfer, order, and stateful examples](examples/test/Choices.t.sol).

For a stateful test, write handlers named `rule_*` and properties named `invariant*`.
The runner calls handlers in generated sequences from several accounts, checks the
invariants along the way, and shrinks a failure to the shortest sequence that causes it.

```solidity
contract CounterTest {
    uint256 value;

    function rule_add(uint8 amount) public { value += amount; }
    function invariant_bounded() public view { assert(value < 1_000); }
}
```

Forge-style invariant tests run as well: a test that deploys handler contracts in
`setUp()` and registers them through `targetContracts()`, or leaves every contract it
deployed as a target, has those contracts' state-changing functions called instead, with
a reverting call treated as a skipped step as under Forge.

`setUp()` runs before every generated case. [`examples/`](examples) is a complete
project with one test per feature.

## Run

```sh
hegel-sol test                                   # the project in the current directory
hegel-sol test --root path/to/project --match-contract Token --match-test transfer
```

A failure prints the decoded revert, its source position, the minimal counterexample
as Solidity, and a command that replays it:

```text
FAIL StatefulCounterTest.stateful
  invariant_accounting_matches (sampled): panic code 0x1
  minimal counterexample:
    vm.prank(0xAC00000000000000000000000000000000000000);
    rule_mint(1);
    vm.prank(0xAC00000000000000000000000000000000000000);
    rule_burn(1);
  reproduce: HEGEL_SOL_REPRODUCE=AXicZYm3... hegel-sol test --root examples \
      --match-contract StatefulCounterTest --match-test stateful
```

The options you will reach first; `hegel-sol test --help` lists the rest.

| Option                                 | Effect                                                            |
| -------------------------------------- | ----------------------------------------------------------------- |
| `--test-cases N`, `--seed N`           | Search budget per test, and a fixed seed.                         |
| `--step-count N`, `--actors N`         | Handler calls per stateful case, and the accounts making them.    |
| `--max-block-advance N`                | Let the engine skip up to N blocks between handler calls.         |
| `--allow-rule-reverts`                 | Skip a reverting handler instead of failing, as Forge does.       |
| `--jobs N`, `--shard K/N`              | Run tests concurrently, or run one stable partition of them.      |
| `--fail-fast`, `--list`                | Stop at the first failure, or only print the selected tests.      |
| `--json`                               | Emit the report documented in [`docs/reporting.md`][reporting].   |
| `--database off`                       | Skip the example database, kept in `.hegel/examples` by default.  |

Failures found earlier replay first from the example database. Generation is guided by
the basic blocks each example reaches; `--no-coverage-target` turns that off.

To check whether a test exercised its intended actions, use `--show-statistics`.
The report separates execution counts, handler outcomes, and counterexample events.
Diagnostics identify handlers that never completed and runs that discarded most inputs,
with suggestions for improving generation. Counts include shrinking and replay; a
handler returning normally does not by itself establish a state change. JSON reports
always include these statistics and diagnostics. See [the reporting contract][reporting]
for exact definitions.

Keep a run for later inspection or CI artifacts with a new output directory:

```sh
hegel-sol test --output-dir .hegel/runs/local-001
hegel-sol report .hegel/runs/local-001 --failed
```

The [run archive](docs/archives.md) includes versions, configuration, the full JSON
report, and failure replay information. Existing directories are never overwritten;
check `manifest.json` for completion or a saved run error.

## Interactive testing

Launch `hegel-sol morgue --root examples` and tell the agent which code you want to
work on. Try “Walk me through ByteMath.average.” The conversation stays open while you
read the source, explore a failing trace, or turn a suggested property into a test.
See the [Morgue guide](docs/morgue.md) for agent setup and a walkthrough.

## Foundry compatibility

Supported cheatcodes: `prank`, `startPrank`, `stopPrank`, `deal`, `warp`, `roll`,
`getBlockNumber`, `getBlockTimestamp`, `expectRevert`, `expectEmit`, `label`, `assume`,
`addr`, `sign`, `store`, `load`, and the `assert*` family forge-std delegates to.
Invariant tests may register `targetContracts()`, `excludeContracts()`,
`targetSenders()`, `excludeSenders()`, and `targetSelectors()`. Selector registrations
restrict the callable functions on each named target; repeated registrations combine their
selectors. Selector registrations also work without `targetContracts()`. Classic DSTest
assertions, which record a failure instead of reverting, are read back with their logged
diagnostics. `console.log` output and custom errors are decoded, and test contracts are
exempt from the code-size limit as in Forge. Forked state is not supported: this is a
standalone runner, not a Foundry fork. Solady's own ERC20 suite and invariant, and
pinned OpenZeppelin, Solady, and Uniswap v2 properties, run under both Forge and
hegel-sol in CI; the [validation record][real-world] has the results.

The [inventory correctness campaign][correctness]
checks finding, shrinking, and replaying a deliberately seeded stale-total error, with
independent Forge confirmation and a corrected passing control.

## More

- [`CHANGELOG.md`](CHANGELOG.md): what changed for users in each release.
- [`docs/design.md`](docs/design.md): how the runner is built and why.
- [`AGENTS.md`](AGENTS.md): conventions, workflows, and how to release.

[releases]: https://github.com/patrickZWY/hegel-sol/releases/latest
[reporting]: docs/reporting.md
[real-world]: crates/tests-integration/projects/real-world/README.md

[correctness]: crates/tests-integration/projects/correctness/README.md
