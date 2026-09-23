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
