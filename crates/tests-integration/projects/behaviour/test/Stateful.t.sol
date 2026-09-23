// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice An invariant that fails with a custom error, so the report has one
/// to decode and attribute.
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

/// @notice A handler that rejects inputs by reverting, the way a real one guards
/// its preconditions.
contract RevertingRuleTest {
    uint256 public sum;

    function rule_add_even(uint8 amount) public {
        require(amount % 2 == 0, "odd amount");
        sum += amount;
    }

    function invariant_sum_stays_even() public view {
        assert(sum % 2 == 0);
    }
}
