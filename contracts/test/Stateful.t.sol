// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "../src/Hegel.sol";

contract StatefulCounterTest is HegelTest {
    uint256 public balance;
    uint256 public accounted;

    function rule_mint(uint8 amount) public {
        balance += amount;
        accounted += amount;
    }

    function rule_burn(uint8 amount) public {
        hegel.assume(amount <= balance);
        balance -= amount;
        // Deliberate bug: burning one token forgets to update the accounting total.
        if (amount != 1) accounted -= amount;
    }

    function invariant_accounting_matches() public view {
        assert(balance == accounted);
    }

    function stepCount() public pure returns (uint256) {
        return 20;
    }
}

/// @notice Pools let a handler reuse a value an earlier handler created, instead
/// of drawing a fresh one that no state refers to.
contract PoolStatefulTest is HegelTest {
    uint256 private pool;
    mapping(bytes32 => bool) private remembered;
    bytes32 private lastPicked;
    bool private picked;

    function setUp() public {
        pool = hegel.poolNew("values");
    }

    function rule_remember(bytes32 value) public {
        hegel.poolAdd(pool, value);
        remembered[value] = true;
    }

    function rule_pick() public {
        lastPicked = hegel.poolPick(pool, false);
        picked = true;
    }

    /// @notice A pick only ever returns a value some earlier call added.
    /// @dev The engine picks by variable id and the runner holds the values, so
    /// a mismatch between the two would surface here as a value this contract
    /// never saw. Picking from an empty pool rejects the step instead, which is
    /// why `picked` can still be false.
    function invariant_picks_come_from_the_pool() public view {
        assert(!picked || remembered[lastPicked]);
    }
}

contract InvariantErrorTest {
    error BrokenInvariant(uint256 actual);

    function rule_noop() public pure {}

    function invariant_custom_error() public pure {
        revert BrokenInvariant(7);
    }

    function stepCount() public pure returns (uint256) {
        return 1;
    }
}
