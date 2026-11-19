// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {HegelTest} from "hegel-sol/Hegel.sol";

/// @notice Minimal ledger for the constrained-transfer example.
contract ChoiceToken {
    mapping(address => uint256) public balanceOf;

    constructor(address owner, uint256 supply) {
        balanceOf[owner] = supply;
    }

    function transfer(address to, uint256 amount) external {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
    }
}

/// @notice Choose distinct actors and an affordable amount without rejecting inputs.
contract ChoiceTransferTest is HegelTest {
    address[] internal actors;
    ChoiceToken internal token;

    function setUp() public {
        actors.push(address(0xA11CE));
        actors.push(address(0xB0B));
        token = new ChoiceToken(actors[0], 100);
    }

    function test_transfer() public {
        address from = pick("from", actors);
        address to = pickOther("to", actors, from);
        uint256 amount = hegel.drawUint256("amount", 0, token.balanceOf(from));
        uint256 beforeBalance = token.balanceOf(to);
        vm.prank(from);
        token.transfer(to, amount);
        assert(token.balanceOf(to) == beforeBalance + amount);
        assert(token.balanceOf(actors[0]) + token.balanceOf(actors[1]) == 100);
    }
}

/// @notice Compose a domain generator as an ordinary Solidity function returning a struct.
contract ChoiceOrderTest is HegelTest {
    struct Order {
        uint256 price;
        uint256 quantity;
    }

    struct Market {
        uint256 price;
        uint256 available;
    }

    function drawOrder(uint256 budget) internal returns (Order memory order) {
        Market[] memory markets = new Market[](2);
        markets[0] = Market(5, 20);
        markets[1] = Market(25, 50);
        Market memory market = markets[pickIndex("market", markets.length)];
        order.price = market.price;
        uint256 affordable = budget / market.price;
        uint256 maximum = affordable < market.available ? affordable : market.available;
        order.quantity = hegel.drawUint256("order.quantity", 1, maximum);
    }

    function test_affordable_order() public {
        Order memory order = drawOrder(1000);
        assert(order.price > 0 && order.quantity > 0);
        assert(order.price * order.quantity <= 1000);
    }
}

/// @notice Select existing objects from current state, including after earlier removals.
contract ChoiceObjectsTest is HegelTest {
    uint256[] internal active;
    mapping(uint256 => bool) internal live;
    uint256 internal nextId;
    uint256 internal lastClosed;
    bool internal closed;

    function rule_open() public {
        uint256 id = ++nextId;
        active.push(id);
        live[id] = true;
    }

    function rule_close() public {
        // No live objects is a normal precondition, not an incorrectly configured choice.
        if (active.length == 0) return;
        uint256 id = pick("object", active);
        assert(live[id]);
        for (uint256 i; i < active.length; i++) {
            if (active[i] != id) continue;
            active[i] = active[active.length - 1];
            active.pop();
            break;
        }
        live[id] = false;
        lastClosed = id;
        closed = true;
    }

    function invariant_closed_objects_are_not_live() public view {
        assert(!closed || !live[lastClosed]);
    }
}
