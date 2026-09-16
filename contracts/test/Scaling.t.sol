// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract ScalingTest {
    function testFuzz_configured_limits(bytes calldata data, uint8[] calldata values) public pure {
        assert(data.length <= 4);
        assert(values.length <= 2);
    }

    function test_parallel_a() public pure {}

    function test_parallel_b() public pure {}

    function test_parallel_c() public pure {}
}

contract FailFastTest {
    function test_00_fails() public pure {
        assert(false);
    }

    function test_01_would_pass() public pure {}

    function test_02_would_also_pass() public pure {}
}

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
