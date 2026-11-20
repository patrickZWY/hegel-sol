// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface SeededVm {
    function envBytes(string calldata name) external returns (bytes memory);
}

/// @notice Models a deliberately stale inventory total after removing items.
contract SeededInventoryHandler {
    uint256 public items;
    uint256 public recordedTotal;
    bool private immutable corrected;

    constructor(bool useCorrection) {
        corrected = useCorrection;
    }

    function add(uint8 amount) public {
        items += amount;
        recordedTotal += amount;
    }

    function remove(uint8 rawAmount) public {
        uint256 amount = uint256(rawAmount) % (items + 1);
        items -= amount;
        // The seeded variant forgets to update the recorded total.
        if (corrected) recordedTotal -= amount;
    }

    function balanced() public view returns (bool) {
        return items == recordedTotal;
    }
}

/// @notice Gives Forge and hegel-sol the same two inventory actions and invariant.
contract SeededInventoryInvariantTest {
    SeededInventoryHandler public handler;

    function setUp() public virtual {
        handler = new SeededInventoryHandler(false);
    }

    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1);
        targets[0] = address(handler);
    }

    function invariant_total() public view {
        assert(handler.balanced());
    }
}

/// @notice Independently confirms the exact archived action arguments under Forge.
contract SeededInventoryReplayTest {
    SeededVm private constant vm = SeededVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function test_archived_sequence() public {
        bytes memory actions = vm.envBytes("HEGEL_INVENTORY_ACTIONS");
        require(actions.length > 0 && actions.length % 2 == 0, "invalid action encoding");
        SeededInventoryHandler handler = new SeededInventoryHandler(false);
        for (uint256 i; i < actions.length; i += 2) {
            assert(handler.balanced());
            uint8 action = uint8(actions[i]);
            uint8 amount = uint8(actions[i + 1]);
            if (action == 0) handler.add(amount);
            else if (action == 1) handler.remove(amount);
            else revert("unknown action");
        }
        assert(!handler.balanced());
    }
}

/// @notice Checks that the corrected inventory total survives the same generated actions.
contract CorrectInventoryInvariantTest is SeededInventoryInvariantTest {
    function setUp() public override {
        handler = new SeededInventoryHandler(true);
    }
}
