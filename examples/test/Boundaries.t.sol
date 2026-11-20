// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice A two-slot stack mistakenly accepts a third push.
contract BoundedStackTest {
    uint8 private size;

    function rule_push() public {
        if (size <= 2) size++;
    }

    function rule_pop() public {
        if (size > 0) size--;
    }

    function invariant_size() public view {
        assert(size <= 2);
    }
}

/// @notice Widen before adding so averaging two bytes cannot overflow.
contract ByteMath {
    function average(uint8 a, uint8 b) public pure returns (uint8) {
        return uint8((uint16(a) + b) / 2);
    }
}

/// @notice Checks rounded byte averages against a wide reference calculation.
contract AverageTest is ByteMath {
    function test_average(uint8 a, uint8 b) public pure {
        assert(average(a, b) == (uint256(a) + b) / 2);
    }
}
