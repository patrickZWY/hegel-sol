// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "hegel-sol/Hegel.sol";

/// @notice A contract whose calls revert with a custom error past a limit and
/// otherwise emit an event and return a value.
contract Guarded {
    error TooMuch(uint256 amount);

    event Taken(address indexed by, uint256 amount);

    function take(uint256 amount) external returns (bool) {
        if (amount > 5) revert TooMuch(amount);
        emit Taken(msg.sender, amount);
        return true;
    }
}

/// @notice A matched `vm.expectRevert` must leave the call's declared return
/// value decodable, or the test reverts while reading it and is reported as a
/// failure Forge would not report.
contract ExpectRevertReturnTest is HegelTest {
    Guarded guarded = new Guarded();

    function test_expected_revert_returns_a_decodable_value() public {
        vm.expectRevert(Guarded.TooMuch.selector);
        bool taken = guarded.take(10);
        assert(!taken);
    }
}

/// @notice `vm.expectEmit` passes when the next call emits a matching event,
/// honours the topic flags, and fails the example when the event never comes.
contract ExpectEmitTest is HegelTest {
    Guarded guarded = new Guarded();

    event Taken(address indexed by, uint256 amount);

    function test_matching_event_passes(uint8 amount) public {
        amount %= 6;
        vm.expectEmit(true, true, true, true);
        emit Taken(address(this), amount);
        guarded.take(amount);
    }

    function test_unchecked_topics_are_ignored(uint8 amount) public {
        amount %= 6;
        vm.expectEmit(false, false, false, true);
        emit Taken(address(0xdead), amount);
        guarded.take(amount);
    }

    function test_missing_event_fails() public {
        vm.expectEmit(true, true, true, true);
        emit Taken(address(this), 1);
        guarded.take(2);
    }
}

/// @notice `vm.sign` produces a signature that `ecrecover` attributes to the
/// address `vm.addr` derives, and a known key derives its known address.
contract SigningTest is HegelTest {
    uint256 constant ORDER = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141;

    function test_known_key_derives_the_known_address() public pure {
        assert(vm.addr(1) == 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf);
    }

    function test_signature_recovers_to_the_derived_address(uint256 key, bytes32 digest) public {
        vm.assume(key != 0 && key < ORDER);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        assert(ecrecover(digest, v, r, s) == vm.addr(key));
    }
}

/// @notice Overloaded test functions are reported under their signatures, so
/// a fixed case and its fuzzed twin stay distinguishable.
contract OverloadedTest {
    function test_same() public pure {}

    function test_same(uint8 value) public pure {
        assert(value <= type(uint8).max);
    }
}

/// @notice `vm.getBlockNumber` and `vm.getBlockTimestamp` read back what `roll`
/// and `warp` set, and agree with what the EVM reports.
contract BlockReadTest is HegelTest {
    function test_block_reads_follow_roll_and_warp() public {
        vm.roll(77);
        vm.warp(12_345);
        assert(vm.getBlockNumber() == 77);
        assert(vm.getBlockTimestamp() == 12_345);
        assert(vm.getBlockNumber() == block.number);
        assert(vm.getBlockTimestamp() == block.timestamp);
    }
}
