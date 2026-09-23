// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

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
