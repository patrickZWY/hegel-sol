// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "hegel-sol/Hegel.sol";

/// @notice Choice helpers expose named values, shrink by candidate order, and diagnose empty inputs.
contract ChoiceHelpersTest is HegelTest {
    error ChoiceReached();

    function test_address_order() public {
        address[] memory candidates = new address[](3);
        candidates[0] = address(0xCAFE);
        candidates[1] = address(0);
        candidates[2] = address(0xB0B);
        address value = pick("actor", candidates);
        assert(value == candidates[0] || value == candidates[1] || value == candidates[2]);
        revert ChoiceReached();
    }

    function test_integer_boundary() public {
        uint256[] memory candidates = new uint256[](3);
        candidates[0] = type(uint256).max;
        candidates[1] = 7;
        candidates[2] = 0;
        uint256 value = pick("id", candidates);
        assert(value != 7);
    }

    function test_singletons() public {
        uint256[] memory integers = new uint256[](1);
        assert(pick("zero", integers) == 0);
        integers[0] = type(uint256).max;
        assert(pick("maximum", integers) == type(uint256).max);
        address[] memory addresses = new address[](1);
        assert(pick("zero address", addresses) == address(0));
        addresses[0] = address(type(uint160).max);
        assert(pick("maximum address", addresses) == address(type(uint160).max));
        revert ChoiceReached();
    }

    function test_other_removes_every_excluded_entry() public {
        address[] memory candidates = new address[](4);
        candidates[0] = address(1);
        candidates[1] = address(1);
        candidates[2] = address(2);
        candidates[3] = address(3);
        address value = pickOther("recipient", candidates, address(1));
        assert(value == address(2) || value == address(3));
        assert(candidates.length == 4 && candidates[0] == address(1) && candidates[1] == address(1));
        revert ChoiceReached();
    }

    function test_other_keeps_zero_when_not_excluded() public {
        address[] memory candidates = new address[](1);
        assert(pickOther("recipient", candidates, address(1)) == address(0));
        revert ChoiceReached();
    }

    function test_empty_addresses() public {
        pick("actors", new address[](0));
    }

    function test_empty_integers() public {
        pick("objects", new uint256[](0));
    }

    function test_empty_index() public {
        pickIndex("orders", 0);
    }

    function test_all_excluded() public {
        address[] memory candidates = new address[](2);
        candidates[0] = address(1);
        candidates[1] = address(1);
        pickOther("recipient", candidates, address(1));
    }
}

/// @notice Replaying a choice rebuilds a collection changed by earlier state-machine actions.
contract ChoiceStateReplayTest is HegelTest {
    error ChoiceReached();
    uint256[] private active;
    bool private removed;

    function rule_open() public {
        if (active.length != 0 || removed) return;
        active.push(11);
        active.push(22);
    }

    function rule_remove() public {
        if (active.length != 2) return;
        active.pop();
        removed = true;
    }

    function rule_choose() public {
        if (!removed) return;
        uint256 id = pick("live object", active);
        assert(id == 11);
        revert ChoiceReached();
    }

    function invariant_collection() public view {
        assert(!removed || active.length == 1);
    }
}
