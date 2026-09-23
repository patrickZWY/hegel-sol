// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Ordinary Forge-style fuzz tests, whose arguments the runner fills in
/// from the ABI with no Hegel calls in the body.
/// @dev Reaching the body at all is most of what these assert: a wrongly encoded
/// argument fails in Solidity's ABI decoder before the first statement runs. The
/// assertions that follow cover the bounds the runner promises to respect.
contract AutoDrawTest {
    struct Pair {
        uint16 count;
        bool enabled;
    }

    function testFuzz_scalars(
        uint256 unsignedValue,
        int128 signedValue,
        bool flag,
        address account,
        bytes32 fixedData,
        bytes calldata data,
        string calldata text
    ) public pure {
        // Consume every argument so the optimizer cannot erase the ABI decoding
        // this test exists to exercise.
        bytes32 witness = keccak256(abi.encode(unsignedValue, signedValue, flag, account, fixedData, data, text));
        assert(witness != bytes32(0));
        assert(data.length <= 64);
        assert(bytes(text).length <= 64);
    }

    function testFuzz_arrays(uint8[] calldata values, address[2] calldata accounts) public pure {
        assert(values.length <= 8);
        // A fixed-size array is always full, so a short one would mean the
        // runner generated the wrong shape rather than the wrong values.
        assert(accounts.length == 2);
    }

    function testFuzz_tuple(Pair calldata pair) public pure {
        bytes32 witness = keccak256(abi.encode(pair.count, pair.enabled));
        assert(witness != bytes32(0));
    }
}
