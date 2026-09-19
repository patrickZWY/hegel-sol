// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Independent tests used to exercise scheduling: parallel execution,
/// ordering, and sharding.
/// @dev These pass under any generation settings on purpose, so a scheduling
/// test never has to care how the runner was configured.
contract ScalingTest {
    function test_parallel_a() public pure {}

    function test_parallel_b() public pure {}

    function test_parallel_c() public pure {}
}

/// @notice Asserts the bounds the runner was told to generate within.
/// @dev Kept apart from ScalingTest because it only passes when the run sets
/// --max-byte-len and --max-array-len to match.
contract GenerationLimitsTest {
    function testFuzz_configured_limits(bytes calldata data, uint8[] calldata values) public pure {
        assert(data.length <= 4);
        assert(values.length <= 2);
    }
}

/// @notice Fails on its first test so fail-fast has something to stop at.
/// @dev Discovery follows the ABI's alphabetical function order, so the numeric
/// prefixes are what put the failing test first.
contract FailFastTest {
    function test_00_fails() public pure {
        assert(false);
    }

    function test_01_would_pass() public pure {}

    function test_02_would_also_pass() public pure {}
}

/// @notice A stepCount() that mutates state, to prove the runner probes it
/// against a throwaway copy of the contract.
contract StepCountIsolationTest {
    uint256 public probes;

    function stepCount() public returns (uint256) {
        probes++;
        return 1;
    }

    function test_probe_does_not_mutate_base() public view {
        assert(probes == 0);
    }

    function rule_noop() public pure {}

    function invariant_probe_does_not_mutate_base() public view {
        assert(probes == 0);
    }
}

/// @notice Workload for scripts/benchmark.sh: wide generated collections.
contract BenchmarkCollectionsTest {
    function testFuzz_large_collections(bytes calldata data, uint256[] calldata values) public pure {
        bytes32 witness = keccak256(abi.encode(data, values));
        assert(witness != bytes32(0));
    }
}

/// @notice Workload for scripts/benchmark.sh: deep state-machine runs.
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
