// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "hegel-sol/Hegel.sol";

/// @notice Case accounting distinguishes completion, assumptions, and found failures.
contract ExampleStatisticsTest is HegelTest {
    function test_completed(uint8 value) public pure {
        assert(uint256(value) < 256);
    }

    function test_filtered(uint8 value) public {
        hegel.assume(value < 128);
        assert(value < 128);
    }

    function test_failure(uint8 value) public pure {
        assert(value != 0);
    }
}

/// @notice Handler accounting keeps normal returns, assumptions, and skipped reverts separate.
contract HandlerStatisticsTest is HegelTest {
    uint256 private calls;

    function rule_complete() public {
        calls++;
    }

    function rule_reject() public {
        hegel.assume(false);
    }

    function rule_revert() public pure {
        revert("unmet handler precondition");
    }

    function invariant_calls() public view {
        assert(calls < type(uint256).max);
    }
}

/// @notice A machine that only skips reverts must identify its unexercised handler.
contract SkippedActionStatisticsTest {
    function rule_skip() public pure {
        revert("always unavailable");
    }

    function invariant_fixture() public pure {}
}

/// @notice A normal return is counted without claiming that business state changed.
contract IdleStatisticsTest {
    function rule_idle() public pure {}

    function invariant_fixture() public pure {}
}

/// @notice An invariant failure before scheduling leaves declared handlers visible with zero calls.
contract UncalledStatisticsTest {
    function rule_unreached() public pure {}

    function invariant_fixture() public pure {
        assert(false);
    }
}
