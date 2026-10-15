// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HEVM, HegelTest} from "hegel-sol/Hegel.sol";

/// @notice The assertion cheatcodes forge-std delegates to. Declared here rather
/// than in Hegel.sol because suites built on forge-std bring their own `Vm`.
interface IAssertions {
    function assertEq(uint256 left, uint256 right) external pure;
    function assertLe(uint256 left, uint256 right, string calldata err) external pure;
    function assertApproxEqAbs(uint256 left, uint256 right, uint256 maxDelta) external pure;
}

/// @notice forge-std's `assertEq` and friends call `vm.assert*`, which must fail
/// the test with a readable message and pass silently otherwise.
contract VmAssertTest {
    IAssertions constant asserts = IAssertions(HEVM);

    function test_passing_assertions_are_silent(uint8 value) public pure {
        asserts.assertLe(value, 255, "a uint8 fits");
        asserts.assertApproxEqAbs(value, value, 0);
    }

    function test_failing_assertion_reverts_with_its_message(uint8 value) public pure {
        asserts.assertEq(value, 7);
    }
}

/// @notice Classic DSTest records a failure in the cheatcode address's storage
/// and returns normally; the runner must read that flag back and report the
/// logged diagnostics.
contract DSTestStyleTest is HegelTest {
    event log(string);
    event log_named_uint(string key, uint256 val);

    function failed() public view returns (bool) {
        return vm.load(address(vm), bytes32("failed")) != bytes32(0);
    }

    function test_recorded_failure_is_reported(uint8 value) public {
        if (value != 7) {
            emit log("Error: value == 7 not satisfied");
            emit log_named_uint("  Actual", value);
            vm.store(address(vm), bytes32("failed"), bytes32(uint256(1)));
        }
    }
}
