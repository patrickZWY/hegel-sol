// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

address constant HEGEL = address(uint160(uint256(keccak256("hegel.sol"))));
address constant HEVM = address(uint160(uint256(keccak256("hevm cheat code"))));

// Version of the runner protocol this file implements. Projects keep their own
// copy of this file, so it can drift from the runner driving it. `HegelTest`
// reports this value through `hegelProtocolVersion()`, and the runner refuses to
// run a contract whose version it does not speak. Bump it whenever a selector or
// the meaning behind one changes.
uint256 constant HEGEL_PROTOCOL_VERSION = 1;

interface IHegel {
    /// @notice Draw an auto-named unsigned integer in the inclusive range [lo, hi].
    function drawUint256(uint256 lo, uint256 hi) external returns (uint256);
    /// @notice Draw an unsigned integer in the inclusive range [lo, hi].
    function drawUint256(string calldata name, uint256 lo, uint256 hi) external returns (uint256);

    /// @notice Draw an auto-named signed integer in the inclusive range [lo, hi].
    function drawInt256(int256 lo, int256 hi) external returns (int256);
    /// @notice Draw a signed integer in the inclusive range [lo, hi].
    function drawInt256(string calldata name, int256 lo, int256 hi) external returns (int256);

    /// @notice Draw an auto-named boolean with equal probability.
    function drawBool() external returns (bool);
    /// @notice Draw a boolean with equal probability.
    function drawBool(string calldata name) external returns (bool);

    /// @notice Draw an auto-named address.
    function drawAddress() external returns (address);
    /// @notice Draw an address.
    function drawAddress(string calldata name) external returns (address);

    /// @notice Draw 32 auto-named bytes.
    function drawBytes32() external returns (bytes32);
    /// @notice Draw exactly 32 bytes.
    function drawBytes32(string calldata name) external returns (bytes32);

    /// @notice Draw auto-named bytes with an inclusive length range.
    function drawBytes(uint256 minLen, uint256 maxLen) external returns (bytes memory);
    /// @notice Draw bytes with an inclusive length range.
    function drawBytes(string calldata name, uint256 minLen, uint256 maxLen) external returns (bytes memory);

    /// @notice Draw an auto-named string no longer than maxLen bytes.
    function drawString(uint256 maxLen) external returns (string memory);
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

    /// @notice Guide generation using a named score. Scores recorded under
    /// different labels are maximised independently.
    function target(string calldata label, int256 score) external;

    /// @notice Record a label for end-of-run statistics.
    function recordEvent(string calldata label) external;

    /// @notice Create a variable pool and return its case-local identifier.
    function poolNew(string calldata name) external returns (uint256 id);

    /// @notice Add a value to a variable pool.
    function poolAdd(uint256 id, bytes32 value) external;

    /// @notice Pick a previously added value, optionally consuming it.
    function poolPick(uint256 id, bool consume) external returns (bytes32 value);
}

/// @notice Foundry-compatible cheatcodes supported by the hegel-sol runner.
interface IHegelVm {
    /// @notice Set msg.sender for the next external call.
    function prank(address sender) external;
    /// @notice Set msg.sender for subsequent external calls.
    function startPrank(address sender) external;
    /// @notice Clear a persistent prank.
    function stopPrank() external;
    /// @notice Set an account's native balance.
    function deal(address account, uint256 balance) external;
    /// @notice Set block.timestamp.
    function warp(uint256 timestamp) external;
    /// @notice Set block.number.
    function roll(uint256 number) external;
    /// @notice Require the next external call to revert.
    function expectRevert() external;
    /// @notice Require the next external call to revert with matching data.
    function expectRevert(bytes calldata revertData) external;
    /// @notice Associate a diagnostic label with an address.
    function label(address account, string calldata label) external;
    /// @notice Reject the current generated example unless condition is true.
    function assume(bool condition) external;
}

abstract contract HegelTest {
    IHegel internal constant hegel = IHegel(HEGEL);
    IHegelVm internal constant vm = IHegelVm(HEVM);

    /// @notice Protocol version this test was compiled against.
    /// @dev The runner calls this before executing anything so a mismatched copy
    /// of Hegel.sol is reported by name instead of as an unknown selector.
    function hegelProtocolVersion() public pure returns (uint256) {
        return HEGEL_PROTOCOL_VERSION;
    }
}
