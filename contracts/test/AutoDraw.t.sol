// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

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
        // Touch every argument so the optimizer cannot erase ABI decoding.
        if (flag && account == address(0)) {
            assert(unsignedValue <= type(uint256).max);
        }
        assert(signedValue >= type(int128).min);
        assert(fixedData.length == 32);
        assert(data.length <= 64);
        assert(bytes(text).length <= 64);
    }

    function testFuzz_arrays(uint8[] calldata values, address[2] calldata accounts) public pure {
        assert(values.length <= 8);
        assert(accounts.length == 2);
    }

    function testFuzz_tuple(Pair calldata pair) public pure {
        assert(pair.count <= type(uint16).max);
    }
}
