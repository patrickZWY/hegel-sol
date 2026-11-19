// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "hegel-sol/Hegel.sol";

/// @notice Unsatisfiable assumptions remain run errors and include execution diagnostics.
contract RejectedExamplesTest is HegelTest {
    function test_reject(uint256 value) public {
        hegel.assume(value == 0);
        hegel.assume(value != 0);
    }
}

/// @notice Excessive filtering can abort before the runner's advisory threshold is reached.
contract OverfilteredExamplesTest is HegelTest {
    function test_filter(uint8 value) public {
        hegel.assume(value < 16);
    }
}
