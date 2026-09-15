# hegel-sol

Property-based testing for Solidity, built on [Hegel](https://hegel.dev) (the Hypothesis
engine behind hegel-rust, hegel-go, hegel-typescript and friends).

Experimental. Not affiliated with the Hegel or Solidity teams.

## Idea

Solidity test contracts draw values through a magic address:

```solidity
import {HegelTest} from "hegel-sol/Hegel.sol";

contract TokenTest is HegelTest {
    function test_transfer_preserves_supply() public {
        address from = hegel.address("from");
        address to = hegel.address("to");
        uint256 amount = hegel.uint256("amount", 0, token.balanceOf(from));
        hegel.assume(from != to);
        token.transfer(from, to, amount);
        assert(token.balanceOf(from) + token.balanceOf(to) == token.totalSupply());
    }
}
```

A Rust runner executes the contract on revm, intercepts those calls, and forwards them to
libhegel, which does generation, shrinking, the example database and reproduce blobs.
Failures come back as a minimal counterexample plus a blob you can replay.

## Layout

- `DESIGN.md`: architecture and the exact engine / revm API facts the runner relies on.
- `PLAN.md`: phased implementation plan with a done-when check per step.
- `crates/hegel-sol`: the Rust runner (skeleton only so far).
- `contracts/`: `Hegel.sol` and sample tests (to be created in Phase 0).

## Status

Research complete, implementation starts at `PLAN.md` step 0.1.
