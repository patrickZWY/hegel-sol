// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice A fixture whose setUp always reverts.
/// @dev setUp runs before every generated example and does not depend on the
/// draws, so its failure says nothing about the property under test.
contract BrokenSetupTest {
    function setUp() public pure {
        revert("fixture setUp is broken");
    }

    function test_property() public pure {}
}

/// @notice A fixture built against a Hegel.sol the runner does not speak.
/// @dev Declaring the accessor directly, rather than importing a doctored copy
/// of Hegel.sol, keeps the skew to the one value under test.
contract StaleProtocolTest {
    function hegelProtocolVersion() public pure returns (uint256) {
        return 99;
    }

    function test_never_reached() public pure {}
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

/// @notice A fuzz test that always fails, so the runner always has a
/// counterexample to report.
/// @dev Only failures carry a trace, so asserting how generated values are
/// reported needs a test that reliably produces one.
contract TracedArgumentsTest {
    function testFuzz_reports_every_argument(uint8 amount, address account, bytes32 salt) public pure {
        // Consume the arguments so solc cannot drop the ABI decoding this
        // fixture exists to report on, then fail.
        bytes32 witness = keccak256(abi.encode(amount, account, salt));
        if (witness != bytes32(0)) revert("always fails");
    }
}
