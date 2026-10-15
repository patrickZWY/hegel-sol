// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Workload for `cargo x bench`: wide generated collections.
contract BenchmarkCollectionsTest {
    function testFuzz_large_collections(bytes calldata data, uint256[] calldata values) public pure {
        bytes32 witness = keccak256(abi.encode(data, values));
        assert(witness != bytes32(0));
    }
}

/// @notice Workload for `cargo x bench`: deep state-machine runs.
contract BenchmarkStatefulTest {
    uint256 public value;
    uint256 private steps;

    function rule_add(uint8 amount) public {
        value += amount;
        steps++;
    }

    function rule_subtract(uint8 amount) public {
        if (amount <= value) value -= amount;
        steps++;
    }

    function invariant_value_never_exceeds_the_amounts_added() public view {
        // Every handler moves `value` by at most 255, so the running total
        // bounds it. A runner that replayed or duplicated a step would break it.
        assert(value <= steps * 255);
    }
}
