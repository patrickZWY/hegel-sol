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

contract PoolStatefulTest is HegelTest {
    uint256 private pool;
    uint256 public additions;
    bytes32 public lastPicked;

    function setUp() public {
        pool = hegel.poolNew("values");
    }

    function rule_remember(bytes32 value) public {
        hegel.poolAdd(pool, value);
        additions++;
    }

    function rule_pick() public {
        lastPicked = hegel.poolPick(pool, false);
    }

    function invariant_pool_is_initialized() public view {
        assert(pool == 0);
    }
}
