// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

address constant HEGEL = address(uint160(uint256(keccak256("hegel.sol"))));

interface IHegel {
    /// @notice Draw an unsigned integer in the inclusive range [lo, hi].
    function drawUint256(string calldata name, uint256 lo, uint256 hi) external returns (uint256);

    /// @notice Draw a signed integer in the inclusive range [lo, hi].
    function drawInt256(string calldata name, int256 lo, int256 hi) external returns (int256);

    /// @notice Draw a boolean with equal probability.
    function drawBool(string calldata name) external returns (bool);

    /// @notice Draw an address.
    function drawAddress(string calldata name) external returns (address);

    /// @notice Draw exactly 32 bytes.
    function drawBytes32(string calldata name) external returns (bytes32);

    /// @notice Draw bytes with an inclusive length range.
    function drawBytes(string calldata name, uint256 minLen, uint256 maxLen) external returns (bytes memory);

    /// @notice Draw a string no longer than maxLen bytes.
    function drawString(string calldata name, uint256 maxLen) external returns (string memory);

    /// @notice Reject the current example unless condition is true.
    function assume(bool condition) external;

    /// @notice Open a group that the shrinker can treat as one unit.
    function startSpan(string calldata label) external;

    /// @notice Close the current group, optionally rejecting it.
    function stopSpan(bool discard) external;

    /// @notice Attach diagnostic text to the current example.
    function note(string calldata text) external;

    /// @notice Guide generation toward examples with larger scores.
    function target(int256 score) external;

    /// @notice Record a label for end-of-run statistics.
    function recordEvent(string calldata label) external;
}

abstract contract HegelTest {
    IHegel internal constant hegel = IHegel(HEGEL);
}
