// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "hegel-sol/Hegel.sol";

/// @notice Remembers who called it, so a test can check which accounts the
/// runner made calls from.
contract Register {
    mapping(address => bool) public seen;

    function poke() external {
        seen[msg.sender] = true;
    }
}

/// @notice Forge-style sender registration: handlers are called only from the
/// accounts `targetSenders()` lists, minus those `excludeSenders()` lists. The
/// fixture also leaves a `startPrank` open, which Forge applies to the calls a
/// test makes and never to the transaction itself, so it must not replace the
/// sender the runner chose. The invariant fails once both allowed accounts
/// have called, so the reported trace shows which senders the runner used.
contract TargetSendersTest is HegelTest {
    address internal constant ALICE = address(0xA11CE);
    address internal constant BOB = address(0xB0B);
    address internal constant BANNED = address(0xBAD);
    address internal constant PRANKED = address(0xDEAD);

    Register internal register;
    address[] internal senders;
    address[] internal excluded;

    function setUp() public {
        register = new Register();
        senders.push(ALICE);
        senders.push(BOB);
        senders.push(BANNED);
        excluded.push(BANNED);
        vm.startPrank(PRANKED);
    }

    function targetSenders() public view returns (address[] memory) {
        return senders;
    }

    function excludeSenders() public view returns (address[] memory) {
        return excluded;
    }

    function invariant_only_registered_senders_call() public view {
        assert(!register.seen(BANNED));
        assert(!register.seen(PRANKED));
        assert(!(register.seen(ALICE) && register.seen(BOB)));
    }
}
