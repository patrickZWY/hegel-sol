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

For a stateful test, write handlers named `rule_*` and properties named `invariant_*`.
The runner calls handlers in generated sequences from several accounts, checks the
invariants along the way, and shrinks a failure to the shortest sequence that causes it.

```solidity
contract CounterTest {
    uint256 value;

    function rule_add(uint8 amount) public { value += amount; }
    function invariant_bounded() public view { assert(value < 1_000); }
}
```

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
| `--allow-rule-reverts`                 | Skip a reverting handler instead of failing, as Forge does.       |
| `--jobs N`, `--shard K/N`              | Run tests concurrently, or run one stable partition of them.      |
| `--fail-fast`, `--list`                | Stop at the first failure, or only print the selected tests.      |
| `--json`                               | Emit the report documented in [`docs/reporting.md`][reporting].   |
| `--database off`                       | Skip the example database, kept in `.hegel/examples` by default.  |

Failures found earlier replay first from the example database. Generation is guided by
the basic blocks each example reaches; `--no-coverage-target` turns that off.

## Foundry compatibility

Supported cheatcodes: `prank`, `startPrank`, `stopPrank`, `deal`, `warp`, `roll`,
`expectRevert`, `label`, and `assume`. `console.log` output and custom errors are
decoded. Forked state is not supported: this is a standalone runner, not a Foundry fork.
Pinned OpenZeppelin, Solady, and Uniswap v2 properties run under both Forge and
hegel-sol in CI; the [validation record][real-world] has the results.

## More

- [`CHANGELOG.md`](CHANGELOG.md): what changed for users in each release.
- [`docs/design.md`](docs/design.md): how the runner is built and why.
- [`AGENTS.md`](AGENTS.md): conventions, workflows, and how to release.

[releases]: https://github.com/patrickZWY/hegel-sol/releases/latest
[reporting]: docs/reporting.md
[real-world]: crates/tests-integration/projects/real-world/README.md
