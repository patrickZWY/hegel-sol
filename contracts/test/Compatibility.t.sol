// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "../src/Hegel.sol";

interface IConsole {
    function log(string calldata message) external view;
}

contract CompatibilityTest is HegelTest {
    address private seenSender;

    function captureSender() external {
        seenSender = msg.sender;
    }

    function alwaysReverts() external pure {
        revert("expected");
    }

    function test_vm_compatibility() public {
        vm.deal(address(this), 7 ether);
        assert(address(this).balance == 7 ether);

        vm.warp(12345);
        assert(block.timestamp == 12345);
        vm.roll(77);
        assert(block.number == 77);

        vm.prank(address(0xBEEF));
        this.captureSender();
        assert(seenSender == address(0xBEEF));

        vm.expectRevert(bytes(""));
        this.alwaysReverts();
        vm.label(address(0xBEEF), "caller");
        vm.assume(true);

        IConsole(0x000000000000000000636F6e736F6c652e6c6f67).log("compatibility works");
    }
}

contract CustomErrorTest is HegelTest {
    error WrongValue(uint256 actual, address caller);

    function test_custom_error() public view {
        IConsole(0x000000000000000000636F6e736F6c652e6c6f67).log("about to fail");
        revert WrongValue(7, msg.sender);
    }
}
