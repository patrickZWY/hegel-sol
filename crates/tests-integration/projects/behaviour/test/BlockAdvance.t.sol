// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice An invariant that holds only while the chain stands still, so the
/// engine's own block advance is the one step that can break it. The clock
/// must move in step with the block number, or the failure is reported with a
/// different reason. Whether the engine may advance at all is left to the
/// runner's option, which is what the test exercises.
contract BlockAdvanceTest {
    uint256 private startBlock;
    uint256 private startTime;

    function setUp() public {
        startBlock = block.number;
        startTime = block.timestamp;
    }

    function rule_noop() public pure {}

    function invariant_time_stands_still() public view {
        require(block.timestamp == startTime + 12 * (block.number - startBlock), "clock drifted");
        assert(block.number == startBlock);
    }
}
